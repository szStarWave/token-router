#!/usr/bin/env python3
"""
Create ModelScope dataset for Token Router OTA if it does not exist.
"""

import os
import sys

OWNER_NAME = "flowy2025"
DATASET_NAME = "token_router_versions"


def get_token() -> str:
    token = os.environ.get("MODELSCOPE_TOKEN", "").strip()
    if not token:
        print("错误: 请设置环境变量 MODELSCOPE_TOKEN", file=sys.stderr)
        sys.exit(1)
    return token


def main() -> None:
    try:
        from modelscope.hub.api import HubApi
    except ImportError:
        print("错误: 需要 modelscope SDK。运行: uv run --with modelscope python ...", file=sys.stderr)
        sys.exit(1)

    repo_id = f"{OWNER_NAME}/{DATASET_NAME}"
    token = get_token()
    api = HubApi()
    api.login(token)

    try:
        api.get_dataset(repo_id)
        print(f"数据集已存在: {repo_id}")
        return
    except Exception:
        pass

    print(f"正在创建数据集: {repo_id}")
    api.create_dataset(
        dataset_name=DATASET_NAME,
        namespace=OWNER_NAME,
        chinese_name="Token Router OTA Versions",
        description="OTA update packages and manifests for Token Router desktop app.",
        license="Apache License 2.0",
    )
    print(f"数据集创建成功: {repo_id}")


if __name__ == "__main__":
    main()
