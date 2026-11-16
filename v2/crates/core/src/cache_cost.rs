//! 缓存损失估计（§9.3）。
//!
//! 定义：换到新候选后，同一 prompt 的等价成本增量。只计算**前缀可复用部分**；
//! 工具结果变化导致的后缀不计入。
//!
//! 未知输入的处理是保守的：缺价格 → `None`（由成本闸按 unknown 策略处理）；
//! 缺缓存数据 → 视为 0 命中（全丢）。

use tr_contracts::{Cost, PriceModel, ReasonCode, ReasonCodeKind};

/// 估计输入。
pub struct CacheInputs<'a> {
    /// 上一次响应实际报告的缓存命中 token；未知填 0。
    pub previous_cached_tokens: u64,
    /// 本次 prompt token 估计。
    pub prompt_tokens: u64,
    /// 当前候选价格。
    pub old_price: &'a PriceModel,
    /// 目标候选价格。
    pub new_price: &'a PriceModel,
    /// 同 provider + 同 model + 前缀哈希匹配时为 true。
    pub same_prefix_reusable: bool,
}

/// 估计结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheLoss {
    pub cost: Cost,
    /// 估算过程中做出的假设（进入 reason codes）。
    pub assumptions: Vec<ReasonCode>,
}

/// 估算切换的缓存损失。币种不一致返回 `None`（不混算）。
pub fn expected_cache_loss(inputs: &CacheInputs<'_>) -> Option<CacheLoss> {
    if inputs.old_price.currency != inputs.new_price.currency {
        return None;
    }

    let prompt = u128::from(inputs.prompt_tokens);
    let cached = if inputs.same_prefix_reusable {
        u128::from(inputs.previous_cached_tokens).min(prompt)
    } else {
        0
    };

    let mut assumptions = Vec::new();

    let old_input = u128::from(inputs.old_price.input_micros_per_mtok);
    let old_cache_read = u128::from(
        inputs
            .old_price
            .cache_read_micros_per_mtok
            .unwrap_or(inputs.old_price.input_micros_per_mtok),
    );
    if inputs.old_price.cache_read_micros_per_mtok.is_none() && cached > 0 {
        assumptions.push(ReasonCode::new(ReasonCodeKind::PriceCacheAssumed));
    }

    let new_input = u128::from(inputs.new_price.input_micros_per_mtok);
    let new_cache_write = u128::from(
        inputs
            .new_price
            .cache_write_micros_per_mtok
            .unwrap_or(inputs.new_price.input_micros_per_mtok),
    );
    let new_supports_cache = inputs.new_price.cache_write_micros_per_mtok.is_some();
    if new_supports_cache && assumptions.is_empty() {
        // 支持缓存但未给价格时已经按 input 价保守估算了。
    }

    // current_cost = cached × old_cache_read + (prompt - cached) × old_input
    let current_cost = (cached * old_cache_read + (prompt - cached) * old_input) / 1_000_000;
    // switch_cost = prompt × new_input + prompt × new_cache_write（仅当新候选支持缓存）
    let switch_cost = if new_supports_cache {
        (prompt * new_input + prompt * new_cache_write) / 1_000_000
    } else {
        (prompt * new_input) / 1_000_000
    };

    let micros = i64::try_from(switch_cost).unwrap_or(i64::MAX)
        - i64::try_from(current_cost).unwrap_or(i64::MAX);

    Some(CacheLoss {
        cost: Cost {
            micros,
            currency: inputs.old_price.currency.clone(),
        },
        assumptions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(input: u64, cache_read: Option<u64>, cache_write: Option<u64>) -> PriceModel {
        PriceModel {
            currency: "USD".to_string(),
            input_micros_per_mtok: input,
            output_micros_per_mtok: 0,
            cache_read_micros_per_mtok: cache_read,
            cache_write_micros_per_mtok: cache_write,
            request_micros: 0,
        }
    }

    #[test]
    fn losing_cache_costs_more() {
        // 旧候选：1M prompt 全命中，缓存价 0.1/M；新候选：无缓存，input 1/M。
        let old = price(1_000_000, Some(100_000), None);
        let new = price(1_000_000, None, None);
        let loss = expected_cache_loss(&CacheInputs {
            previous_cached_tokens: 1_000_000,
            prompt_tokens: 1_000_000,
            old_price: &old,
            new_price: &new,
            same_prefix_reusable: true,
        })
        .expect("loss");

        // current = 100_000；switch = 1_000_000 → loss = 900_000
        assert_eq!(loss.cost.micros, 900_000);
    }

    #[test]
    fn switching_prefix_means_full_loss() {
        let old = price(1_000_000, Some(100_000), None);
        let new = price(1_000_000, None, None);
        let loss = expected_cache_loss(&CacheInputs {
            previous_cached_tokens: 1_000_000,
            prompt_tokens: 1_000_000,
            old_price: &old,
            new_price: &new,
            same_prefix_reusable: false,
        })
        .expect("loss");
        // 前缀不可复用：current = prompt × input = 1M；switch = 1M → loss = 0
        assert_eq!(loss.cost.micros, 0);
    }

    #[test]
    fn cheaper_target_can_have_negative_loss() {
        let old = price(2_000_000, None, None);
        let new = price(1_000_000, None, None);
        let loss = expected_cache_loss(&CacheInputs {
            previous_cached_tokens: 0,
            prompt_tokens: 1_000_000,
            old_price: &old,
            new_price: &new,
            same_prefix_reusable: false,
        })
        .expect("loss");
        assert!(loss.cost.micros < 0);
    }

    #[test]
    fn currency_mismatch_is_unknown() {
        let old = price(1_000_000, None, None);
        let mut new = price(1_000_000, None, None);
        new.currency = "CNY".to_string();
        assert!(
            expected_cache_loss(&CacheInputs {
                previous_cached_tokens: 0,
                prompt_tokens: 100,
                old_price: &old,
                new_price: &new,
                same_prefix_reusable: false,
            })
            .is_none()
        );
    }

    #[test]
    fn missing_cache_price_is_recorded() {
        let old = price(1_000_000, None, None); // 没有 cache_read 价格却声明命中
        let new = price(1_000_000, None, None);
        let loss = expected_cache_loss(&CacheInputs {
            previous_cached_tokens: 500_000,
            prompt_tokens: 1_000_000,
            old_price: &old,
            new_price: &new,
            same_prefix_reusable: true,
        })
        .expect("loss");
        assert!(
            loss.assumptions
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::PriceCacheAssumed)
        );
    }
}
