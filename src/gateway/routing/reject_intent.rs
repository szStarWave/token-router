/// User correction / dissatisfaction keywords (zh / yue / en / ja / ko).
pub fn contains_reject_intent(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    REJECT_KWS.iter().any(|k| {
        if k.is_ascii() {
            lower.contains(k)
        } else {
            text.contains(k)
        }
    })
}

const REJECT_KWS: &[&str] = &[
    // --- Mandarin (zh): negation ---
    "不对",
    "错了",
    "错啦",
    "不正确",
    "不对的",
    "答错了",
    "说错了",
    "搞错了",
    "弄错了",
    "不是这个",
    "不是那样",
    "不是这样",
    "完全不对",
    "全错了",
    "你搞错了",
    "你错了",
  // --- Mandarin (zh): dissatisfaction ---
    "不好",
    "答案不好",
    "回答不好",
    "不满意",
    "不行",
    "不可以",
    "太差了",
    "不靠谱",
  // --- Mandarin (zh): redo ---
    "重新回答",
    "再说一遍",
    "再讲一遍",
    "重来",
    "换一个",
    "重新说",
    "重新讲",
    "再说一次",
    // --- Cantonese (yue): negation ---
    "唔啱",
    "唔岩",
    "错咗",
    "讲错咗",
    "答错咗",
    "唔系咁",
    "唔系啊",
    "唔啱啊",
    "唔得",
    "错晒",
    "唔岩啊",
    "讲错",
    "答错",
    // --- Cantonese (yue): redo ---
    "重新讲过",
    "再讲多次",
    "再答过",
    "讲过",
    "答过",
    "再讲过",
    // --- English (en): negation ---
    "wrong",
    "incorrect",
    "not right",
    "not correct",
    "that's wrong",
    "that is wrong",
    "that's incorrect",
    "that is incorrect",
    "no that's wrong",
    "nope that's wrong",
    "bad answer",
    "poor answer",
    "not good",
    "that's not right",
    "this is wrong",
    "you're wrong",
    "you are wrong",
    // --- English (en): redo ---
    "try again",
    "redo",
    "do it again",
    "answer again",
    "say it again",
    "start over",
    "try that again",
    // --- Japanese (ja): negation ---
    "違う",
    "間違い",
    "間違っている",
    "間違ってる",
    "正しくない",
    "そうじゃない",
    "違います",
    "おかしい",
    "ダメ",
    "違いますよ",
    "ちがう",
    "まちがい",
    "まちがってる",
    "まちがっている",
    // --- Japanese (ja): redo ---
    "もう一度",
    "やり直し",
    "答え直して",
    "言い直して",
    "再度",
    "もういちど",
    // --- Korean (ko): negation ---
    "틀렸",
    "틀려",
    "틀렸어",
    "틀렸습니다",
    "잘못",
    "잘못됐",
    "아니야",
    "아닙니다",
    "아니에요",
    "그게 아니야",
    "그건 아니야",
    "틀린 것 같아",
    "틀린 답",
    "틀렸어요",
    // --- Korean (ko): redo ---
    "다시",
    "다시 해",
    "다시 답해",
    "다시 말해",
    "재답변",
    "다시 해줘",
    "다시 말해줘",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_reject_hits() {
        assert!(contains_reject_intent("不对，重新说"));
        assert!(contains_reject_intent("这个答案不好"));
    }

    #[test]
    fn yue_reject_hits() {
        assert!(contains_reject_intent("唔啱啊，再讲多次"));
    }

    #[test]
    fn en_reject_hits() {
        assert!(contains_reject_intent("that's wrong, try again"));
    }

    #[test]
    fn ja_reject_hits() {
        assert!(contains_reject_intent("違います、もう一度"));
        assert!(contains_reject_intent("ちがうよ"));
    }

    #[test]
    fn ko_reject_hits() {
        assert!(contains_reject_intent("틀렸어, 다시 답해줘"));
    }

    #[test]
    fn neutral_does_not_hit() {
        assert!(!contains_reject_intent("你好，今天天气怎么样"));
        assert!(!contains_reject_intent("啱咗"));
    }
}
