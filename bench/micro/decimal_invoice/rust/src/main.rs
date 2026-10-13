// Invoices on rust_decimal: 200,000 invoices of 1 to 9 lines, a lump-sum discount allocated
// across each invoice's lines in proportion (a division per line, rounded to cents half to
// even), then 8.25% tax per line.
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;

const INVOICES: u32 = 200_000;

fn main() {
    let mut seed: u64 = 7;
    let mut next = || {
        seed = (seed * 48271) % 2147483647;
        seed
    };
    let (mut grand, mut taxes, mut discounts) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let even = RoundingStrategy::MidpointNearestEven;
    for _ in 0..INVOICES {
        let lines = 1 + (next() / 1024) % 9;
        let mut totals: Vec<Decimal> = Vec::new();
        let mut sum = Decimal::ZERO;
        for _ in 0..lines {
            let price = Decimal::from((next() / 1024) % 100000) / dec!(100);
            let qty = 1 + (next() / 1024) % 12;
            let total = price * Decimal::from(qty);
            totals.push(total);
            sum += total;
        }
        if sum.is_zero() {
            continue;
        }
        let discount = Decimal::from((next() / 1024) % 5000) / dec!(100);
        for total in &totals {
            let share = (discount * total / sum).round_dp_with_strategy(2, even);
            let net = total - share;
            let tax = (net * dec!(0.0825)).round_dp_with_strategy(2, even);
            grand += net + tax;
            taxes += tax;
            discounts += share;
        }
    }
    println!("grand total {grand}");
    println!("tax {taxes}");
    println!("discounts {discounts}");
}
