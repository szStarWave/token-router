pub use super::keywords::contains_reject_intent;

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
