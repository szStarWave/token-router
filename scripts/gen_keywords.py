#!/usr/bin/env python3
"""Generate src/gateway/routing/keywords.rs with 7 groups x 5 langs x 64+ terms.

Cognitive intents (ANALYSIS / DECISION / RESEARCH) live in cognitive_intent.rs;
regenerate via scripts/gen_cognitive_intent.py (or gen_cognitive_intent.js).
"""

from __future__ import annotations

import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src" / "gateway" / "routing" / "keywords.rs"

LANGS = ["ZH", "EN", "YUE", "JA", "KO"]

# Each group: 8 clusters x 8 terms per language
TABLES: dict[str, dict[str, list[list[str]]]] = {
    "TOOL_ERROR": {
        "ZH": [
            ["错误", "出错", "出问题", "不成功", "未能", "失败", "异常", "故障"],
            ["超时", "连接失败", "连接超时", "请求超时", "网络错误", "断开", "连不上", "无响应"],
            ["未找到", "找不到", "不存在", "无此文件", "路径错误", "文件缺失", "目录不存在", "无权限访问"],
            ["无权限", "拒绝访问", "权限不足", "未授权", "禁止", "鉴权失败", "认证失败", "访问被拒"],
            ["编译失败", "构建失败", "语法错误", "解析错误", "链接错误", "打包失败", "lint失败", "构建出错"],
            ["崩溃", "致命", "中断", "终止", "段错误", "栈溢出", "内存不足", "panic"],
            ["命令失败", "命令未找到", "执行失败", "退出码", "非零退出", "进程killed", "signal", "执行出错"],
            ["npm err", "yarn error", "cargo error", "python error", "http 404", "http 500", "http 503", "assertion failed"],
        ],
        "EN": [
            ["error", "failed", "failure", "fail", "fault", "errored", "unsuccessful", "unsuccessfully"],
            ["timeout", "timed out", "connection failed", "connection reset", "refused", "timedout", "no response", "network error"],
            ["not found", "no such file", "file not found", "path error", "missing file", "directory", "enoent", "does not exist"],
            ["permission denied", "access denied", "unauthorized", "forbidden", "auth failed", "eacces", "not authorized", "access forbidden"],
            ["compile error", "compilation failed", "build failed", "syntax error", "parse error", "link error", "bundler", "build error"],
            ["panic", "crash", "crashed", "fatal", "aborted", "segfault", "oom", "stack overflow"],
            ["command failed", "command not found", "exit code", "exit status", "non-zero", "killed", "signal", "execution failed"],
            ["npm err", "yarn error", "pnpm err", "cargo error", "python error", "http 404", "http 500", "assertion failed"],
        ],
        "YUE": [
            ["出错", "失败", "异常", "唔成功", "做唔成", "出问题", "故障", "搞唔掂"],
            ["超时", "连唔到", "断线", "无响应", "请求失败", "连接失败", "网络错", "连唔上"],
            ["搵唔到", "唔存在", "路径错", "文件唔见", "无此文件", "目录唔存在", "路径错误", "文件缺失"],
            ["无权限", "拒绝访问", "鉴权失败", "唔俾访问", "权限不足", "未授权", "禁止", "访问被拒"],
            ["编译失败", "构建失败", "语法错", "link失败", "解析错", "打包失败", "lint失败", "构建出错"],
            ["崩溃", "致命", "中断", "内存唔够", "段错误", "栈溢出", "终止", "panic"],
            ["命令失败", "命令唔到", "exit code", "进程被杀", "执行失败", "非零退出", "signal", "执行出错"],
            ["npm 错", "HTTP 500", "assertion 失败", "http 404", "cargo error", "yarn error", "http 503", "python error"],
        ],
        "JA": [
            ["エラー", "失敗", "異常", "故障", "不成功", "失敗しました", "エラーが", "失敗した"],
            ["タイムアウト", "接続失敗", "接続切断", "ネットワークエラー", "接続超时", "応答なし", "切断", "接続エラー"],
            ["見つかりません", "ファイルがない", "パスエラー", "存在しません", "見つからない", "ファイル未找到", "ディレクトリなし", "アクセス拒否"],
            ["権限", "拒否", "認証失敗", "アクセス拒否", "未授权", "禁止", "権限不足", "認証エラー"],
            ["コンパイル失敗", "ビルド失敗", "構文エラー", "リンクエラー", "解析エラー", "打包失敗", "lint失敗", "ビルドエラー"],
            ["クラッシュ", "致命的", "中断", "パanic", "セグフォ", "メモリ不足", "終了", "異常終了"],
            ["コマンド失敗", "コマンド未找到", "終了コード", "シグナル", "実行失敗", "非ゼロ", "killed", "実行エラー"],
            ["npm エラー", "HTTP 404", "HTTP 500", "アサーション失敗", "cargo error", "yarn error", "http 503", "python error"],
        ],
        "KO": [
            ["오류", "실패", "에러", "예외", "고장", "실패함", "오류가", "실패했"],
            ["타임아웃", "연결 실패", "연결 끊", "네트워크 오류", "응답 없음", "연결 거부", "접속 실패", "네트워크 에러"],
            ["찾을 수 없", "파일 없음", "경로 오류", "없습니다", "파일 없", "디렉터리 없", "존재하지", "접근 거부"],
            ["권한", "거부", "인증 실패", "접근 거부", "미인증", "금지", "권한 부족", "인증 오류"],
            ["컴파일 실패", "빌드 실패", "구문 오류", "링크 오류", "파싱 오류", "패키징 실패", "lint 실패", "빌드 오류"],
            ["크래시", "치명", "중단", "panic", "세그폴트", "메모리 부족", "종료", "비정상 종료"],
            ["명령 실패", "명령 없음", "종료 코드", "시그널", "실행 실패", "非零", "killed", "실행 오류"],
            ["npm 오류", "HTTP 404", "HTTP 500", "assertion 실패", "cargo error", "yarn error", "http 503", "python error"],
        ],
    },
    "HARD_INTENT": {
        "ZH": [
            ["分析", "解析", "调研", "研究", "评估", "审查", "剖析", "研判"],
            ["总结", "概括", "对比", "比较", "归纳", "梳理", "对照", "比对"],
            ["优化", "改进", "重构", "重写", "整理", "瘦身", "改良", "重构代码"],
            ["架构", "设计", "建模", "模块划分", "系统设计", "架构设计", "方案设计", "总体设计"],
            ["安全", "审计", "合规", "漏洞", "风险评估", "渗透", "安全审计", "合规审查"],
            ["修复", "调试", "排错", "查bug", "root cause", "故障排查", "定位问题", "排查故障"],
            ["部署", "发布", "上线", "迁移", "集成", "回滚", "发布上线", "部署迁移"],
            ["并发", "分布式", "算法", "证明", "跨仓库", "性能瓶颈", "高并发", "分布式系统"],
        ],
        "EN": [
            ["analyze", "examine", "assess", "evaluate", "investigate", "study", "research", "deep dive"],
            ["summarize", "compare", "contrast", "synthesize", "overview", "comparison", "summarise", "contrastive"],
            ["optimize", "refactor", "rewrite", "restructure", "improve", "streamline", "optimise", "reorganize"],
            ["architect", "design", "model", "system design", "structure", "architecture", "architectural", "modular design"],
            ["security", "audit", "compliance", "vulnerability", "risk", "legal", "medical", "penetration"],
            ["fix", "debug", "troubleshoot", "diagnose", "root cause", "patch", "debugging", "troubleshooting"],
            ["deploy", "release", "migration", "rollout", "integrate", "rollback", "deployment", "migrate"],
            ["concurrent", "distributed", "algorithm", "proof", "cross-repo", "performance", "bottleneck", "scalability"],
        ],
        "YUE": [
            ["分析", "解析", "调研", "研究", "评估", "审查", "剖析", "研判"],
            ["总结", "概括", "对比", "比较", "归纳", "梳理", "对照", "比对"],
            ["优化", "改进", "重构", "重写", "整理", "瘦身", "改良", "重构代码"],
            ["架构", "设计", "建模", "模块划分", "系统设计", "架构设计", "方案设计", "总体设计"],
            ["安全", "审计", "合规", "漏洞", "风险评估", "渗透", "安全审计", "合规审查"],
            ["修复", "调试", "排错", "查bug", "root cause", "故障排查", "定位问题", "排查故障"],
            ["部署", "发布", "上线", "迁移", "集成", "回滚", "发布上线", "部署迁移"],
            ["并发", "分布式", "算法", "证明", "跨仓库", "性能瓶颈", "高并发", "分布式系统"],
        ],
        "JA": [
            ["分析", "解析", "調査", "研究", "評価", "審査", "剖析", "検討"],
            ["要約", "比較", "対比", "整理", "まとめ", "対照", "比較分析", "総括"],
            ["最適化", "改善", "リファクタ", "書き直し", "整理", "改良", "再構築", "最適化する"],
            ["アーキテクチャ", "設計", "モデリング", "システム設計", "構造", "設計する", "モジュール", "全体設計"],
            ["セキュリティ", "監査", "コンプライアンス", "脆弱性", "リスク", "法務", "医療", "ペネトレーション"],
            ["修正", "デバッグ", "トラブルシュート", "診断", "根本原因", "パッチ", "障害調査", "問題解決"],
            ["デプロイ", "リリース", "移行", "ロールアウト", "統合", "ロールバック", "本番", "マイグレーション"],
            ["並行", "分散", "アルゴリズム", "証明", "クロスリポ", "性能", "ボトルネック", "スケーラビリティ"],
        ],
        "KO": [
            ["분석", "해석", "조사", "연구", "평가", "심사", "해부", "검토"],
            ["요약", "비교", "대조", "정리", "종합", "대조 분석", "개요", "총괄"],
            ["최적화", "개선", "리팩터", "재작성", "정리", "개선하다", "재구성", "스트림라인"],
            ["아키텍처", "설계", "모델링", "시스템 설계", "구조", "설계하기", "모듈", "전체 설계"],
            ["보안", "감사", "컴플라이언스", "취약점", "리스크", "법무", "의료", "침투"],
            ["수정", "디버그", "트러블슈트", "진단", "근본 원인", "패치", "장애 조사", "문제 해결"],
            ["배포", "릴리스", "마이그레이션", "롤아웃", "통합", "롤백", "배포하기", "이전"],
            ["동시", "분산", "알고리즘", "증명", "크로스 레포", "성능", "병목", "확장성"],
        ],
    },
    "PLAN_INTENT": {
        "ZH": [
            ["计划", "规划", "方案", "打算", "安排", "筹划", "谋划", "部署计划"],
            ["路线图", "战略", "方针", "策略", "目标", "战略规划", "发展路线", "战略方针"],
            ["步骤", "阶段", "环节", "流程", "次序", "步骤安排", "阶段划分", "流程设计"],
            ["里程碑", "节点", "排期", "时间表", "deadline", "时间节点", "里程碑计划", "进度表"],
            ["拆解", "分解", "拆分", "分任务", "子任务", "任务拆解", "分解任务", "拆分任务"],
            ["执行计划", "实施", "落地", "推进", "执行方案", "实施计划", "落地计划", "推进计划"],
            ["优先级", "待办", "清单", "backlog", "todo", "待办清单", "优先顺序", "任务清单"],
            ["项目计划", "行动计划", "work plan", "action plan", "step-by-step", "项目规划", "行动方案", "分步计划"],
        ],
        "EN": [
            ["plan", "planning", "scheme", "arrange", "schedule", "planner", "planned", "plan out"],
            ["roadmap", "strategy", "strategic", "objective", "strategic plan", "road map", "vision", "direction"],
            ["step", "phase", "stage", "workflow", "sequence", "steps", "phases", "stages"],
            ["milestone", "schedule", "timeline", "deadline", "timeframe", "milestones", "due date", "time line"],
            ["breakdown", "decompose", "split tasks", "subtask", "task breakdown", "decomposition", "split up", "sub tasks"],
            ["implementation", "execute", "rollout plan", "implement", "execution plan", "carry out", "implement plan", "execute plan"],
            ["priority", "backlog", "todo list", "checklist", "priorities", "todo", "task list", "to-do"],
            ["project plan", "action plan", "work plan", "step-by-step", "project planning", "action items", "workplan", "step plan"],
        ],
        "YUE": [
            ["计划", "规划", "方案", "打算", "安排", "筹划", "谋划", "部署计划"],
            ["路线图", "战略", "方针", "策略", "目标", "战略规划", "发展路线", "战略方针"],
            ["步骤", "阶段", "环节", "流程", "次序", "步骤安排", "阶段划分", "流程设计"],
            ["里程碑", "节点", "排期", "时间表", "deadline", "时间节点", "里程碑计划", "进度表"],
            ["拆解", "分解", "拆分", "分任务", "子任务", "任务拆解", "分解任务", "拆分任务"],
            ["执行计划", "实施", "落地", "推进", "执行方案", "实施计划", "落地计划", "推进计划"],
            ["优先级", "待办", "清单", "backlog", "todo", "待办清单", "优先顺序", "任务清单"],
            ["项目计划", "行动计划", "work plan", "action plan", "step-by-step", "项目规划", "行动方案", "分步计划"],
        ],
        "JA": [
            ["計画", "プランニング", "方案", "予定", "手配", "企画", "計画する", "プラン"],
            ["ロードマップ", "戦略", "方針", "策略", "目標", "戦略計画", "路線", "方針策定"],
            ["ステップ", "段階", "フェーズ", "ワークフロー", "順序", "手順", "段階分け", "フロー"],
            ["マイルストーン", "スケジュール", "タイムライン", "締切", "期限", "節目", "進捗表", "期日"],
            ["分解", "タスク分割", "サブタスク", "ブレークダウン", "分割", "タスク分解", "細分化", "割当"],
            ["実行計画", "実施", "実行", "推進", "実装計画", "実施計画", "ロールアウト", "実行する"],
            ["優先度", "バックログ", "todo", "チェックリスト", "優先順位", "タスクリスト", "ToDo", "待ち"],
            ["プロジェクト計画", "アクションプラン", "work plan", "action plan", "step-by-step", "計画立案", "行動計画", "段階計画"],
        ],
        "KO": [
            ["계획", "플래닝", "방안", "예정", "배치", "기획", "계획하다", "플랜"],
            ["로드맵", "전략", "방침", "전략적", "목표", "전략 계획", "로드 맵", "방향"],
            ["단계", "페이즈", "스테이지", "워크플로", "순서", "스텝", "단계별", "흐름"],
            ["마일스톤", "일정", "타임라인", "데드라인", "기한", "절점", "진행표", "마감"],
            ["분해", "태스크 분할", "서브태스크", "브레이크다운", "나누기", "작업 분해", "세분화", "할당"],
            ["실행 계획", "실시", "실행", "추진", "구현 계획", "실행안", "롤아웃", "수행"],
            ["우선순위", "백로그", "todo", "체크리스트", "우선도", "할 일", "태스크 목록", "대기"],
            ["프로젝트 계획", "액션 플랜", "work plan", "action plan", "step-by-step", "계획 수립", "행동 계획", "단계 계획"],
        ],
    },
    "EASY_INTENT": {
        "ZH": [
            ["你好", "您好", "嗨", "早上好", "晚上好", "在吗", "哈喽", "午安"],
            ["谢谢", "多谢", "感谢", "辛苦了", "thanks", "thx", "十分感谢", "非常感谢"],
            ["再见", "拜拜", "回见", "bye", "goodbye", "再会", "下次见", "告辞"],
            ["天气", "几点", "什么时间", "what time", "weather", "现在几点", "什么时间", "气温"],
            ["聊聊", "聊天", "说说", "chat", "small talk", "随便聊", "聊一聊", "唠嗑"],
            ["继续", "还有吗", "然后呢", "go on", "continue", "接着说", "还有呢", "往下说", "再说", "再讲", "再讲一次", "再讲一遍"],
            ["介绍", "什么是", "解释一下", "tell me about", "explain", "介绍一下", "是什么", "讲讲"],
            ["可以吗", "行吗", "好的", "嗯", "没问题", "ok", "no problem", "好吧"],
        ],
        "EN": [
            ["hello", "good morning", "good evening", "hey", "hi there", "howdy", "greetings", "good afternoon"],
            ["thanks", "thank you", "thx", "appreciate", "much appreciated", "thanks a lot", "thank", "grateful"],
            ["goodbye", "bye", "see you", "farewell", "later", "take care", "bye bye", "catch you"],
            ["weather", "what time", "current time", "temperature", "forecast", "time now", "what's the time", "how's weather"],
            ["chat", "small talk", "talk about", "let's chat", "casual chat", "just chat", "chatting", "converse"],
            ["continue", "go on", "what else", "and then", "keep going", "more please", "tell me more", "keep talking"],
            ["tell me about", "explain", "what is", "introduce", "describe", "what are", "can you explain", "define"],
            ["okay", "sure", "no problem", "alright", "fine", "sounds good", "that works", "yes please"],
        ],
        "YUE": [
            ["你好", "您好", "嗨", "早晨", "晚上好", "喺度", "哈喽", "午安"],
            ["多谢", "唔该", "感谢", "辛苦晒", "thanks", "thx", "十分感谢", "非常感谢"],
            ["再见", "拜拜", "回见", "bye", "goodbye", "再会", "下次见", "告辞"],
            ["天气", "几点", "几时", "what time", "weather", "而家几点", "咩时间", "气温"],
            ["倾下", "倾偈", "讲下", "chat", "small talk", "随便倾", "倾一倾", "倾下计"],
            ["继续", "仲有冇", "跟住点", "go on", "continue", "跟住讲", "仲有呢", "往下讲", "再讲", "再讲一次", "再讲过"],
            ["介绍", "咩系", "解释下", "tell me about", "explain", "介绍下", "系咩", "讲下"],
            ["得唔得", "好唔好", "好嘅", "嗯", "冇问题", "ok", "no problem", "好啦"],
        ],
        "JA": [
            ["こんにちは", "おはよう", "こんばんは", "やあ", "どうも", "ハロー", "ご挨拶", "午後"],
            ["ありがとう", "感謝", "サンキュー", "お疲れ", "thanks", "thx", "どうも", "感謝します"],
            ["さようなら", "バイバイ", "またね", "bye", "goodbye", "失礼", "じゃあね", "お疲れ様"],
            ["天気", "何時", "今何時", "what time", "weather", "時間", "気温", "天候"],
            ["話そう", "雑談", "話す", "chat", "small talk", "ちょっと話", "おしゃべり", "会話"],
            ["続けて", "他に", "それで", "go on", "continue", "続き", "もっと", "先へ"],
            ["教えて", "説明", "とは", "tell me about", "explain", "紹介", "何ですか", "意味"],
            ["大丈夫", "いいよ", "問題ない", "ok", "no problem", "了解", "はい", "いいです"],
        ],
        "KO": [
            ["안녕", "안녕하세요", "좋은 아침", "좋은 저녁", "hey", "hi", "인사", "반가워"],
            ["감사", "고마워", "thanks", "thx", "감사합니다", "고맙습니다", "감사해", "고마워요"],
            ["안녕히", "잘 가", "bye", "goodbye", "다음에", "또 봐", "잘 있어", "작별"],
            ["날씨", "몇 시", "지금 몇 시", "what time", "weather", "시간", "기온", "날씨 어때"],
            ["수다", "잡담", "이야기", "chat", "small talk", "대화", "수다 떨", "얘기"],
            ["계속", "더 있어", "그다음", "go on", "continue", "이어서", "더 말해", "계속해"],
            ["알려줘", "설명", "뭐야", "tell me about", "explain", "소개", "무엇", "뜻"],
            ["괜찮", "좋아", "문제없", "ok", "no problem", "네", "알겠", "그래"],
        ],
    },
    "REJECT_INTENT": {
        "ZH": [
            ["不对", "错了", "不正确", "不对的", "答错了", "说错了", "搞错了", "弄错了"],
            ["不好", "答案不好", "回答不好", "不满意", "不行", "太差了", "不靠谱", "质量差"],
            ["重新回答", "再说一遍", "再讲一遍", "重来", "换一个", "重新说", "重新讲", "再说一次"],
            ["不准确", "答非所问", "离题", "文不对题", "偏题", "跑题", "没答到", "答偏了"],
            ["没帮助", "没用", "无帮助", "帮不上", "没有用", "毫无帮助", "不实用", "解决不了"],
            ["胡说", "瞎说", "乱说", "一派胡言", "瞎扯", "乱讲", "无稽之谈", "瞎编"],
            ["换一个", "重新生成", "regenerate", "再生成", "换答案", "重做", "再来", "重新来"],
            ["完全不对", "全错了", "你搞错了", "你错了", "大错特错", "错得离谱", "全错", "错光"],
        ],
        "EN": [
            ["wrong", "incorrect", "not right", "not correct", "that's wrong", "that is wrong", "bad answer", "poor answer"],
            ["not good", "that's not right", "this is wrong", "you're wrong", "you are wrong", "unsatisfactory", "terrible answer", "useless answer"],
            ["try again", "redo", "do it again", "answer again", "say it again", "start over", "try that again", "once more"],
            ["inaccurate", "off topic", "not relevant", "doesn't answer", "missed the point", "irrelevant", "off-topic", "wrong topic"],
            ["unhelpful", "not helpful", "useless", "no help", "doesn't help", "not useful", "worthless", "no use"],
            ["nonsense", "rubbish", "garbage", "bullshit", "absurd", "ridiculous", "hogwash", "baloney"],
            ["regenerate", "another answer", "different answer", "new answer", "generate again", "replace answer", "swap answer", "fresh answer"],
            ["completely wrong", "totally wrong", "all wrong", "way off", "dead wrong", "so wrong", "entirely wrong", "utterly wrong"],
        ],
        "YUE": [
            ["唔啱", "唔岩", "错咗", "讲错咗", "答错咗", "唔系咁", "唔系啊", "唔啱啊"],
            ["唔得", "错晒", "唔岩啊", "答案唔好", "唔满意", "太差", "唔靠谱", "质量差"],
            ["重新讲过", "再讲多次", "再答过", "讲过", "答过", "再讲过", "重来", "换一个"],
            ["唔准确", "答非所问", "离题", "文不对题", "偏题", "跑题", "冇答到", "答偏"],
            ["冇帮助", "冇用", "帮唔到", "无用", "毫无帮助", "唔实用", "解决唔到", "帮唔上忙"],
            ["乱讲", "瞎讲", "胡说", "一派胡言", "乱话", "无稽之谈", "瞎编", "乱噏"],
            ["换一个", "重新生成", "regenerate", "再生成", "换答案", "重做", "再来", "重新来"],
            ["完全唔啱", "全错", "你错咗", "大错特错", "错到离谱", "全错晒", "错光", "错完"],
        ],
        "JA": [
            ["違う", "間違い", "間違っている", "間違ってる", "正しくない", "そうじゃない", "違います", "おかしい"],
            ["ダメ", "違いますよ", "ちがう", "まちがい", "まちがってる", "まちがっている", "不満", "質が悪い"],
            ["もう一度", "やり直し", "答え直して", "言い直して", "再度", "もういちど", "やり直す", "再回答"],
            ["不正確", "的外れ", "話がずれ", "答えになってない", "ズレてる", "的外れ", " irrelevant", "論点外"],
            ["役に立たない", "役立たない", "無駄", "助けにならない", "使えない", "不十分", "参考にならない", "意味ない"],
            ["ナンセンス", "でたらめ", "意味不明", "胡散臭い", "おかしい", "荒唐", "無意味", "デタラメ"],
            ["再生成", "別の答え", "答え直し", "regenerate", "新しい答え", "作り直し", "差し替え", "別回答"],
            ["全く違う", "完全に間違い", "大間違い", "全然違う", "全部間違い", "根本的に違う", "完全におかしい", "全滅"],
        ],
        "KO": [
            ["틀렸", "틀려", "틀렸어", "틀렸습니다", "잘못", "잘못됐", "아니야", "아닙니다"],
            ["아니에요", "그게 아니야", "그건 아니야", "틀린 것 같아", "틀린 답", "틀렸어요", "불만", "품질 나쁨"],
            ["다시", "다시 해", "다시 답해", "다시 말해", "재답변", "다시 해줘", "다시 말해줘", "다시 해봐"],
            ["부정확", "동문서답", "주제 벗어남", "엉뚱", "딴소리", "답변 아님", "관련 없음", "빗나감"],
            ["도움 안", "소용없", "도움 안 돼", "쓸모없", "무용", "도움 안 됨", "별로", "해결 안"],
            ["헛소리", "말도 안", "터무니없", "엉터리", "무의미", "허튼소리", "개소리", "말장난"],
            ["다시 생성", "regenerate", "다른 답", "새 답", "재생성", "답 바꿔", "다시 만들어", "교체"],
            ["완전 틀", "전부 틀", "완전히 틀", "다 틀", "크게 틀", "전혀 아님", "완전히 잘못", "틀림없이 틀"],
        ],
    },
    "UNCERTAINTY": {
        "ZH": [
            ["不确定", "不太确定", "不一定", "未必", "说不准", "难说", "不好确定", "拿不准"],
            ["不知道", "不清楚", "说不清", "不了解", "没把握", "说不明白", "不明", "未知"],
            ["无法确定", "没法确定", "难以判断", "无法判断", "不好判断", "难下定论", "说不死", "定不了"],
            ["不好说", "难说", "说不准", "说不明白", "难讲", "说不来", "讲不好", "说不清"],
            ["也许", "可能", "大概", "估计", "我猜", "或许", "说不定", "或许吧"],
            ["差不多", "大致", "roughly", "kind of", "左右", "约莫", "大约", "大概吧"],
            ["没有把握", "不太清楚", "not confident", "信心不足", "不太肯定", "存疑", "有疑问", "不太信"],
            ["might be", "could be", "可能吧", "或许", "かもしれない", "일 수도", "可能系", "也许吧"],
        ],
        "EN": [
            ["not sure", "unsure", "uncertain", "unclear", "not certain", "hard to say", "can't say", "don't know for sure"],
            ["don't know", "no idea", "not know", "unknown", "can't tell", "no clue", "not aware", "unaware"],
            ["cannot determine", "cannot tell", "hard to determine", "unable to say", "can't determine", "can't decide", "indeterminate", "undetermined"],
            ["hard to say", "difficult to say", "tough to say", "tricky to say", "not easy to say", "hard to tell", "difficult to tell", "can't be sure"],
            ["maybe", "perhaps", "probably", "i guess", "possibly", "might", "could be", "likely maybe"],
            ["roughly", "kind of", "sort of", "more or less", "approximately", "about", "around", "somewhat"],
            ["not confident", "low confidence", "no confidence", "uncertain answer", "tentative", "speculative", "guesswork", "conjecture"],
            ["might be", "could be", "may be", "possibly be", "potentially", "perhaps it", "maybe it", "could possibly"],
        ],
        "YUE": [
            ["唔确定", "唔太确定", "唔一定", "未必", "讲唔准", "难讲", "唔好确定", "拎唔准"],
            ["唔知", "唔清楚", "讲唔清", "唔了解", "冇把握", "讲唔明", "唔明", "未知"],
            ["无法确定", "冇法确定", "难判断", "无法判断", "唔好判断", "难下定论", "讲唔死", "定唔到"],
            ["唔好讲", "难讲", "讲唔准", "讲唔明", "难谂", "讲唔来", "讲唔好", "讲唔清"],
            ["也许", "可能", "大概", "估计", "我估", "或许", "话唔定", "或许啦"],
            ["差唔多", "大致", "roughly", "kind of", "左右", "约莫", "大约", "大概啦"],
            ["冇把握", "唔太清楚", "not confident", "信心唔足", "唔太肯定", "存疑", "有疑问", "唔太信"],
            ["might be", "could be", "可能啦", "或许", "可能系", "话唔定", "也许啦", "或者"],
        ],
        "JA": [
            ["確信がない", "不明", "不確か", "わからない", "はっきりしない", "定かでない", "不明確", "曖昧"],
            ["知らない", "分からない", "不明", "未確定", "把握していない", "説明できない", "未知", "不詳"],
            ["判断できない", "断定できない", "決められない", "特定できない", "確定できない", "言い切れない", "決めかねる", "定められない"],
            ["言いにくい", "言いづらい", "言えない", "言い難い", "言いにく", "言い切れない", "言えません", "言いがち"],
            ["たぶん", "おそらく", "かもしれない", "推測", "多分", "恐らく", "もしかして", "推量"],
            ["だいたい", "およそ", "roughly", "kind of", "程度", "約", "前後", "大体"],
            ["自信がない", "確信がない", "not confident", "低信頼", "不確実", "疑わしい", "疑問", "保留"],
            ["might be", "could be", "かも", "perhaps", "maybe", "possibly", "potential", "may be"],
        ],
        "KO": [
            ["확실하지 않", "불확실", "잘 모르", "애매", "분명하지 않", "확신 없", "불명확", "모호"],
            ["모르겠", "모름", "알 수 없", "미확인", "파악 못", "설명 못", "미지", "불상"],
            ["판단 못", "단정 못", "결정 못", "특정 못", "확정 못", "말 못", "결정하기 어렵", "정하기 어렵"],
            ["말하기 어렵", "말하긴 어렵", "말 못", "말하기 힘들", "말하기 곤란", "말하기 애매", "말 못하", "말하기 난감"],
            ["아마", "어쩌면", "아마도", "추측", "대략", "혹시", "가능성", "짐작"],
            ["대략", "어느 정도", "roughly", "kind of", "정도", "약", "전후", "쯤"],
            ["자신 없", "확신 없", "not confident", "낮은 신뢰", "불확실한", "의심", "의문", "보류"],
            ["might be", "could be", "일 수도", "perhaps", "maybe", "possibly", "potential", "may be"],
        ],
    },
    "SPECIAL_LEXICAL": {
        "ZH": [
            ["GDPR", "HIPAA", "SOX", "PCI-DSS", "等保", "个保法", "金融合规", "KYC"],
            ["Kubernetes", "etcd", "Istio", "service mesh", "Terraform", "Helm", "ArgoCD", "K8s"],
            ["transformer", "attention", "RLHF", "LoRA", "quantization", "RAG", "embedding", "fine-tune"],
            ["CVE", "XSS", "CSRF", "SSRF", "RCE", "OWASP", "penetration", "zero-day"],
            ["smart contract", "merkle", "EVM", "solidity", "DeFi", "NFT", "blockchain", "web3"],
            ["CRISPR", "mRNA", "polymerase", "基因编辑", "基因组", "碱基", "转录", "蛋白质"],
            ["statute", "injunction", "tort", "民法典", "刑法", "判例", "诉讼", "仲裁"],
            ["OpenAI", "Anthropic", "OAuth", "SAML", "WebRTC", "gRPC", "protobuf", "GraphQL"],
        ],
        "EN": [
            ["GDPR", "HIPAA", "SOX", "PCI-DSS", "compliance", "KYC", "AML", "regulatory"],
            ["Kubernetes", "etcd", "Istio", "service mesh", "Terraform", "Helm", "ArgoCD", "K8s"],
            ["transformer", "attention", "RLHF", "LoRA", "quantization", "RAG", "embedding", "fine-tune"],
            ["CVE", "XSS", "CSRF", "SSRF", "RCE", "OWASP", "penetration", "zero-day"],
            ["smart contract", "merkle", "EVM", "solidity", "DeFi", "NFT", "blockchain", "web3"],
            ["CRISPR", "mRNA", "polymerase", "genome", "genomic", "transcript", "protein", "allele"],
            ["statute", "injunction", "tort", "precedent", "litigation", "arbitration", "subpoena", "indictment"],
            ["OpenAI", "Anthropic", "OAuth", "SAML", "WebRTC", "gRPC", "protobuf", "GraphQL"],
        ],
        "YUE": [
            ["GDPR", "HIPAA", "SOX", "PCI-DSS", "等保", "个保法", "金融合规", "KYC"],
            ["Kubernetes", "etcd", "Istio", "service mesh", "Terraform", "Helm", "ArgoCD", "K8s"],
            ["transformer", "attention", "RLHF", "LoRA", "quantization", "RAG", "embedding", "fine-tune"],
            ["CVE", "XSS", "CSRF", "SSRF", "RCE", "OWASP", "penetration", "zero-day"],
            ["smart contract", "merkle", "EVM", "solidity", "DeFi", "NFT", "blockchain", "web3"],
            ["CRISPR", "mRNA", "polymerase", "基因编辑", "基因组", "碱基", "转录", "蛋白质"],
            ["statute", "injunction", "tort", "民法典", "刑法", "判例", "诉讼", "仲裁"],
            ["OpenAI", "Anthropic", "OAuth", "SAML", "WebRTC", "gRPC", "protobuf", "GraphQL"],
        ],
        "JA": [
            ["GDPR", "HIPAA", "SOX", "PCI-DSS", "コンプライアンス", "KYC", "AML", "規制"],
            ["Kubernetes", "etcd", "Istio", "service mesh", "Terraform", "Helm", "ArgoCD", "K8s"],
            ["transformer", "attention", "RLHF", "LoRA", "quantization", "RAG", "embedding", "fine-tune"],
            ["CVE", "XSS", "CSRF", "SSRF", "RCE", "OWASP", "penetration", "zero-day"],
            ["smart contract", "merkle", "EVM", "solidity", "DeFi", "NFT", "blockchain", "web3"],
            ["CRISPR", "mRNA", "polymerase", "ゲノム", "遺伝子編集", "塩基", "転写", "タンパク質"],
            ["statute", "injunction", "tort", "判例", "訴訟", "仲裁", "民法", "刑法"],
            ["OpenAI", "Anthropic", "OAuth", "SAML", "WebRTC", "gRPC", "protobuf", "GraphQL"],
        ],
        "KO": [
            ["GDPR", "HIPAA", "SOX", "PCI-DSS", "컴플라이언스", "KYC", "AML", "규제"],
            ["Kubernetes", "etcd", "Istio", "service mesh", "Terraform", "Helm", "ArgoCD", "K8s"],
            ["transformer", "attention", "RLHF", "LoRA", "quantization", "RAG", "embedding", "fine-tune"],
            ["CVE", "XSS", "CSRF", "SSRF", "RCE", "OWASP", "penetration", "zero-day"],
            ["smart contract", "merkle", "EVM", "solidity", "DeFi", "NFT", "blockchain", "web3"],
            ["CRISPR", "mRNA", "polymerase", "유전자", "게놈", "염기", "전사", "단백질"],
            ["statute", "injunction", "tort", "판례", "소송", "중재", "민법", "형법"],
            ["OpenAI", "Anthropic", "OAuth", "SAML", "WebRTC", "gRPC", "protobuf", "GraphQL"],
        ],
    },
}

FUNC_MAP = {
    "TOOL_ERROR": "tool_result_has_error",
    "HARD_INTENT": "contains_hard_intent",
    "PLAN_INTENT": "contains_plan_intent",
    "EASY_INTENT": "contains_easy_intent",
    "REJECT_INTENT": "contains_reject_intent",
    "UNCERTAINTY": "response_has_uncertainty",
    "SPECIAL_LEXICAL": "contains_special_lexical",
}


def flatten(clusters: list[list[str]]) -> list[str]:
    out: list[str] = []
    for c in clusters:
        out.extend(c)
    return out


def emit_array(name: str, terms: list[str]) -> str:
    lines = [f"const {name}: &[&str] = &["]
    for t in terms:
        esc = t.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    "{esc}",')
    lines.append("];")
    lines.append(f"const _: () = assert!({name}.len() >= 64);")
    return "\n".join(lines)


def main() -> None:
    parts: list[str] = [
        "//! Centralized routing keyword tables (7 groups x 5 languages, >=64 terms each).",
        "",
        "/// Match keywords: ASCII terms are case-insensitive; CJK/YUE/JA/KO use raw substring.",
        "pub fn text_matches_keywords(text: &str, keywords: &[&str]) -> bool {",
        "    let lower = text.to_ascii_lowercase();",
        "    keywords.iter().any(|k| {",
        "        if k.is_ascii() {",
        "            lower.contains(k)",
        "        } else {",
        "            text.contains(k)",
        "        }",
        "    })",
        "}",
        "",
        "fn matches_any_lang(text: &str, tables: &[&[&str]]) -> bool {",
        "    tables.iter().any(|t| text_matches_keywords(text, t))",
        "}",
        "",
    ]

    const_names: dict[str, dict[str, str]] = {}

    for group, langs in TABLES.items():
        parts.append(f"// --- {group} ---")
        tables_for_fn: list[str] = []
        for lang in LANGS:
            terms = flatten(langs[lang])
            assert len(terms) >= 64, f"{group}_{lang} has {len(terms)}"
            cname = f"{group}_{lang}"
            const_names.setdefault(group, {})[lang] = cname
            parts.append(emit_array(cname, terms))
            parts.append("")
            tables_for_fn.append(cname)
        fn = FUNC_MAP[group]
        if group == "PLAN_INTENT":
            parts.append(f"pub fn {fn}(text: &str) -> bool {{")
            parts.append(f"    let lower = text.to_ascii_lowercase();")
            parts.append(f"    let trimmed = lower.trim();")
            parts.append(
                f"    (trimmed.starts_with(\"plan \") || trimmed == \"plan\")"
            )
            parts.append(
                f"        || matches_any_lang(text, &[{', '.join('&'+t for t in tables_for_fn)}])"
            )
            parts.append("}")
        else:
            parts.append(f"pub fn {fn}(text: &str) -> bool {{")
            parts.append(
                f"    matches_any_lang(text, &[{', '.join('&'+t for t in tables_for_fn)}])"
            )
            parts.append("}")
        parts.append("")

    parts.append("#[cfg(test)]")
    parts.append("mod tests {")
    parts.append("    use super::*;")
    parts.append("")
    parts.append("    #[test]")
    parts.append("    fn keyword_table_minimum_sizes() {")
    for group, langs in const_names.items():
        for lang, cname in langs.items():
            parts.append(f"        assert!({cname}.len() >= 64);")
    parts.append("    }")
    parts.append("")
    parts.append("    #[test]")
    parts.append("    fn tool_error_smoke() {")
    parts.append('        assert!(tool_result_has_error("Error: command failed"));')
    parts.append('        assert!(tool_result_has_error("失败: exit code 1"));')
    parts.append("    }")
    parts.append("")
    parts.append("    #[test]")
    parts.append("    fn uncertainty_cascade_gate() {")
    parts.append('        assert!(response_has_uncertainty("I\'m not sure about that"));')
    parts.append('        assert!(response_has_uncertainty("わからない"));')
    parts.append('        assert!(response_has_uncertainty("唔确定"));')
    parts.append('        assert!(response_has_uncertainty("아마"));')
    parts.append("    }")
    parts.append("")
    parts.append("    #[test]")
    parts.append("    fn special_lexical_smoke() {")
    parts.append('        assert!(contains_special_lexical("Configure Kubernetes ingress"));')
    parts.append('        assert!(contains_special_lexical("GDPR compliance audit"));')
    parts.append("    }")
    parts.append("}")

    OUT.write_text("\n".join(parts) + "\n", encoding="utf-8")
    print(f"Wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
