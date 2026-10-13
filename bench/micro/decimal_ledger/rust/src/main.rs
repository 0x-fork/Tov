// The payments ledger on rust_decimal: 2,000,000 charges parsed from text, a 2.9% + 0.30 fee
// rounded to cents (half to even), net per merchant, payouts at 0.9234.
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;
use std::str::FromStr;

const N: u32 = 2_000_000;
const MERCHANTS: usize = 1000;

fn main() {
    let mut seed: u64 = 42;
    let mut next = || {
        seed = (seed * 48271) % 2147483647;
        seed
    };
    let mut gross = vec![Decimal::ZERO; MERCHANTS];
    let mut net = vec![Decimal::ZERO; MERCHANTS];
    let mut fees = Decimal::ZERO;
    for _ in 0..N {
        let cents = (next() / 64) % 500000;
        let text = format!("{}.{:02}", cents / 100, cents % 100);
        let amount = Decimal::from_str(&text).unwrap();
        let fee = (amount * dec!(0.029) + dec!(0.30)).round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
        let m = ((next() / 1024) % MERCHANTS as u64) as usize;
        gross[m] += amount;
        net[m] += amount - fee;
        fees += fee;
    }
    let (mut total_gross, mut total_net, mut payout) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for i in 0..MERCHANTS {
        total_gross += gross[i];
        total_net += net[i];
        payout += (net[i] * dec!(0.9234)).round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
    }
    println!("gross {total_gross}");
    println!("fees {fees}");
    println!("net {total_net}");
    println!("payout EUR {payout}");
    println!("merchant 7: {} gross, {} net", gross[7], net[7]);
}
