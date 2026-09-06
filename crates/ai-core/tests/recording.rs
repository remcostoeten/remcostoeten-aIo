//! Accounting arithmetic, extracted unchanged.
//!
//! The estimation and cost formulas are compatibility surface: Skriuw's stored
//! history was computed with them, so changing either silently reprices old
//! runs relative to new ones.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ai_core::{
    AiModelPrice, AiRunTokens, AiTokenSource, AiUsage, MAX_AI_TOKEN_COUNT, ai_run_cost_micros,
    estimate_ai_tokens,
};

#[test]
fn estimates_tokens_as_ceiling_of_bytes_over_four() {
    assert_eq!(estimate_ai_tokens(0), 0);
    assert_eq!(estimate_ai_tokens(1), 1);
    assert_eq!(estimate_ai_tokens(4), 1);
    assert_eq!(estimate_ai_tokens(5), 2);
    assert_eq!(estimate_ai_tokens(4_000), 1_000);
}

#[test]
fn caps_an_estimate_at_the_token_bound() {
    assert_eq!(estimate_ai_tokens(usize::MAX), MAX_AI_TOKEN_COUNT);
}

#[test]
fn reported_tokens_keep_provider_provenance() {
    let tokens = AiRunTokens::reported(&AiUsage {
        input_tokens: 7,
        output_tokens: 9,
    });
    assert_eq!(tokens.source, AiTokenSource::Provider);
    assert_eq!((tokens.input_tokens, tokens.output_tokens), (7, 9));
}

#[test]
fn estimated_tokens_keep_estimated_provenance() {
    let tokens = AiRunTokens::estimated(10, 5);
    assert_eq!(tokens.source, AiTokenSource::Estimated);
    assert_eq!((tokens.input_tokens, tokens.output_tokens), (3, 2));
}

#[test]
fn prices_a_run_rounding_half_up() {
    let price = AiModelPrice {
        input_price_micros_per_mtok: 150_000,
        output_price_micros_per_mtok: 600_000,
    };
    let tokens = AiRunTokens::reported(&AiUsage {
        input_tokens: 1_000_000,
        output_tokens: 500_000,
    });
    assert_eq!(ai_run_cost_micros(&tokens, price), 450_000);
}

#[test]
fn rounds_a_half_micro_upwards() {
    let price = AiModelPrice {
        input_price_micros_per_mtok: 1,
        output_price_micros_per_mtok: 0,
    };
    let half = AiRunTokens::reported(&AiUsage {
        input_tokens: 500_000,
        output_tokens: 0,
    });
    assert_eq!(ai_run_cost_micros(&half, price), 1);

    let just_under = AiRunTokens::reported(&AiUsage {
        input_tokens: 499_999,
        output_tokens: 0,
    });
    assert_eq!(ai_run_cost_micros(&just_under, price), 0);
}

#[test]
fn computes_intermediate_products_beyond_the_javascript_safe_range() {
    // 1e9 tokens at 1e9 micros per million: the intermediate product is 1e18,
    // roughly a hundred times past Number.MAX_SAFE_INTEGER, while the result is
    // a comfortable 1e12. A port that multiplies in doubles gets a plausible
    // wrong answer here, not an error, so exact intermediates are mandatory.
    let price = AiModelPrice {
        input_price_micros_per_mtok: 1_000_000_000,
        output_price_micros_per_mtok: 0,
    };
    let tokens = AiRunTokens::reported(&AiUsage {
        input_tokens: MAX_AI_TOKEN_COUNT,
        output_tokens: 0,
    });
    assert_eq!(ai_run_cost_micros(&tokens, price), 1_000_000_000_000);
}

#[test]
fn saturates_rather_than_overflowing_on_absurd_prices() {
    let price = AiModelPrice {
        input_price_micros_per_mtok: u64::MAX,
        output_price_micros_per_mtok: u64::MAX,
    };
    let tokens = AiRunTokens::reported(&AiUsage {
        input_tokens: MAX_AI_TOKEN_COUNT,
        output_tokens: MAX_AI_TOKEN_COUNT,
    });
    assert_eq!(ai_run_cost_micros(&tokens, price), u64::MAX);
}
