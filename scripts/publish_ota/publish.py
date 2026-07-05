#!/usr/bin/env python3
"""
OTA publish script — upload Token Router Windows NSIS setup to ModelScope dataset.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time

from modelscope.hub.api import HubApi


OWNER_NAME = "flowy2025"
DATASET_NAME = "token_router_versions"
REPO_ROOT = Path(__file__).resolve().parents[2]
UPLOAD_RETRIES = 3
UPLOAD_RETRY_DELAY_SECS = 5


def parse_args():
    default_release_notes = REPO_ROOT / "docs" / "ota-release-notes.json"
    parser = argparse.ArgumentParser(description="OTA publish script for Token Router")
    parser.add_argument("--channel", type=str, default="flowy", help="Channel (flowy/gmk)")
    parser.add_argument("--region-scope", type=str, default="CN", help="Region scope (CN/INTL)")
    parser.add_argument("--version", type=str, required=True, help="Version tag")
    parser.add_argument("--enable-account-system", type=str, default="true", help="true/false")
    parser.add_argument(
        "--setup-path",
        "--exe-path",
        dest="setup_path",
        type=str,
        required=True,
        help="Path to NSIS setup installer (legacy alias: --exe-path)",
    )
    parser.add_argument(
        "--release-notes-file",
        type=str,
        default=str(default_release_notes),
        help="Release notes JSON path",
    )
    parser.add_argument(
        "--manifest-only",
        action="store_true",
        help="Upload latest.json only (recover when setup was already published)",
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


def upload_folder_with_retry(
    api: HubApi,
    folder_path: str,
    path_in_repo: str,
    repo_id: str,
    commit_message: str,
) -> None:
    last_error: Exception | None = None
    for attempt in range(1, UPLOAD_RETRIES + 1):
        try:
            print(
                f"正在上传目录: {folder_path} -> {path_in_repo} "
                f"(attempt {attempt}/{UPLOAD_RETRIES})"
            )
            api.upload_folder(
                repo_id=repo_id,
                folder_path=folder_path,
                path_in_repo=path_in_repo,
                repo_type="dataset",
                commit_message=commit_message,
            )
            print(f"上传成功: {path_in_repo}")
            return
        except Exception as exc:
            last_error = exc
            print(f"上传失败 (attempt {attempt}/{UPLOAD_RETRIES}): {exc}", file=sys.stderr)
            if attempt < UPLOAD_RETRIES:
                print(f"等待 {UPLOAD_RETRY_DELAY_SECS}s 后重试...", file=sys.stderr)
                time.sleep(UPLOAD_RETRY_DELAY_SECS)
    raise last_error if last_error is not None else RuntimeError("upload failed")


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


def write_latest_json(
    staging_dir: Path,
    version: str,
    setup_filename: str,
    release_notes: dict,
) -> None:
    latest_json = {
        "version": version if version.startswith("v") else f"v{version}",
        "file": setup_filename,
        "release_notes": release_notes,
    }
    latest_path = staging_dir / "latest.json"
    with latest_path.open("w", encoding="utf-8") as f:
        json.dump(latest_json, f, indent=4, ensure_ascii=False)


def main() -> None:
    args = parse_args()

    if not args.manifest_only and not os.path.exists(args.setup_path):
        print(f"错误: setup 安装包不存在: {args.setup_path}", file=sys.stderr)
        sys.exit(1)

    setup_filename = os.path.basename(args.setup_path)
    print("开始发布 Token Router OTA 更新...")
    print(f"Channel: {args.channel}")
    print(f"RegionScope: {args.region_scope}")
    print(f"Version: {args.version}")
    print(f"EnableAccountSystem: {args.enable_account_system}")
    print(f"SetupPath: {args.setup_path}")
    print(f"SetupFilename: {setup_filename}")
    print(f"ReleaseNotesFile: {args.release_notes_file}")
    print(f"ManifestOnly: {args.manifest_only}")

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
    path_in_repo = f"{args.region_scope}/{args.channel}/{account_dir}"
    manifest_version = args.version if args.version.startswith("v") else f"v{args.version}"

    staging_dir = Path(tempfile.mkdtemp(prefix="token_router_ota_publish_"))
    try:
        write_latest_json(staging_dir, manifest_version, setup_filename, release_notes)
        if not args.manifest_only:
            shutil.copy2(args.setup_path, staging_dir / setup_filename)

        commit_message = (
            f"upload manifest: {manifest_version}"
            if args.manifest_only
            else f"upload OTA: {manifest_version}"
        )
        upload_folder_with_retry(
            api,
            str(staging_dir),
            path_in_repo,
            repo_id,
            commit_message,
        )
    finally:
        shutil.rmtree(staging_dir, ignore_errors=True)

    print("OTA 发布完成!")


if __name__ == "__main__":
    main()
