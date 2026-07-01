#!/usr/bin/env python3
"""
OTA publish script — upload Token Router Windows builds to ModelScope dataset.
"""

import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile

from modelscope.hub.api import HubApi


OWNER_NAME = "flowy2025"
DATASET_NAME = "token_router_versions"


def parse_args():
    default_release_notes = Path(__file__).resolve().parents[3] / "docs" / "ota-release-notes.json"
    parser = argparse.ArgumentParser(description="OTA publish script for Token Router")
    parser.add_argument("--channel", type=str, default="flowy", help="Channel (flowy/gmk)")
    parser.add_argument("--region-scope", type=str, default="CN", help="Region scope (CN/INTL)")
    parser.add_argument("--version", type=str, required=True, help="Version tag")
    parser.add_argument("--enable-account-system", type=str, default="true", help="true/false")
    parser.add_argument("--exe-path", type=str, required=True, help="Path to release exe")
    parser.add_argument(
        "--release-notes-file",
        type=str,
        default=str(default_release_notes),
        help="Release notes JSON path",
    )
    return parser.parse_args()


def get_token() -> str:
    token = os.environ.get("MODELSCOPE_TOKEN", "").strip()
    if not token:
        print("错误: 请设置环境变量 MODELSCOPE_TOKEN", file=sys.stderr)
        sys.exit(1)
    return token


def get_repo_id() -> str:
    return f"{OWNER_NAME}/{DATASET_NAME}"


def upload_file(api: HubApi, local_path: str, path_in_repo: str, repo_id: str) -> None:
    print(f"正在上传: {local_path} -> {path_in_repo}")
    api.upload_file(
        path_or_fileobj=local_path,
        path_in_repo=path_in_repo,
        repo_id=repo_id,
        repo_type="dataset",
        commit_message=f"upload: {path_in_repo}",
    )
    print(f"上传成功: {path_in_repo}")


def release_notes_lookup_keys(version: str) -> list[str]:
    keys = [version]
    match = re.match(r"^(v?\d+\.\d+\.\d+)-\d+-g[0-9a-fA-F]+$", version.strip())
    if match:
        keys.append(match.group(1))
    if not version.startswith("v") and re.match(r"^\d+\.\d+\.\d+", version):
        keys.append(f"v{version}")
    return keys


def load_release_notes(path: str, version: str) -> dict:
    notes_path = Path(path)
    if not notes_path.exists():
        raise FileNotFoundError(f"release notes 文件不存在: {notes_path}")

    with notes_path.open("r", encoding="utf-8") as f:
        doc = json.load(f)

    versions = doc.get("versions")
    if not isinstance(versions, dict):
        raise ValueError("release notes 文件必须包含对象字段: versions")

    matched_version = ""
    notes = None
    for key in release_notes_lookup_keys(version):
        notes = versions.get(key)
        if notes is not None:
            matched_version = key
            break
    if notes is None:
        tried = ", ".join(release_notes_lookup_keys(version))
        raise KeyError(f"release notes 缺少版本 {version}，已尝试: {tried}")
    if not isinstance(notes, dict):
        raise ValueError(f"release notes 版本 {matched_version} 必须是对象")

    normalized = {}
    for lang, items in notes.items():
        if not isinstance(lang, str) or not isinstance(items, list):
            raise ValueError(f"release notes 版本 {matched_version} 的语言条目格式错误: {lang}")
        normalized_items = []
        for item in items:
            if not isinstance(item, str):
                raise ValueError(f"release notes 版本 {matched_version} 的条目必须是字符串")
            text = item.strip()
            if text:
                normalized_items.append(text)
        if normalized_items:
            normalized[lang] = normalized_items

    if not normalized:
        raise ValueError(f"release notes 版本 {matched_version} 不能为空")
    if matched_version != version:
        print(f"ReleaseNotesVersion: {matched_version} (fallback for {version})")
    else:
        print(f"ReleaseNotesVersion: {matched_version}")
    return normalized


def create_latest_json(version: str, exe_filename: str, release_notes: dict) -> str:
    latest_json = {
        "version": version if version.startswith("v") else f"v{version}",
        "file": exe_filename,
        "release_notes": release_notes,
    }
    fd, path = tempfile.mkstemp(suffix=".json", prefix="latest_")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(latest_json, f, indent=4, ensure_ascii=False)
    return path


def main() -> None:
    args = parse_args()

    if not os.path.exists(args.exe_path):
        print(f"错误: exe 文件不存在: {args.exe_path}", file=sys.stderr)
        sys.exit(1)

    exe_filename = os.path.basename(args.exe_path)
    print("开始发布 Token Router OTA 更新...")
    print(f"Channel: {args.channel}")
    print(f"RegionScope: {args.region_scope}")
    print(f"Version: {args.version}")
    print(f"EnableAccountSystem: {args.enable_account_system}")
    print(f"ExePath: {args.exe_path}")
    print(f"ExeFilename: {exe_filename}")
    print(f"ReleaseNotesFile: {args.release_notes_file}")

    try:
        release_notes = load_release_notes(args.release_notes_file, args.version)
    except Exception as e:
        print(f"错误: 读取 OTA 更新内容失败: {e}", file=sys.stderr)
        sys.exit(1)

    api = HubApi()
    api.login(get_token())

    repo_id = get_repo_id()
    print(f"目标数据集: {repo_id}")

    account_dir = "with_account" if args.enable_account_system == "true" else "without_account"

    path_in_repo = f"{args.region_scope}/{args.channel}/{account_dir}/{exe_filename}"
    upload_file(api, args.exe_path, path_in_repo, repo_id)

    manifest_version = args.version if args.version.startswith("v") else f"v{args.version}"
    latest_json_path = create_latest_json(manifest_version, exe_filename, release_notes)
    try:
        latest_path_in_repo = f"{args.region_scope}/{args.channel}/{account_dir}/latest.json"
        upload_file(api, latest_json_path, latest_path_in_repo, repo_id)
    finally:
        os.unlink(latest_json_path)

    print("OTA 发布完成!")


if __name__ == "__main__":
    main()
