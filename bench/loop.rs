// loop.rs — 2,700,000-iteration counter loop. Prints 3644998650000.
fn main() {
    let mut n = 0.0;
    let mut total = 0.0;
    while n < 2700000.0 {
        total += n;
        n += 1.0;
    }
    println!("{:.0}", total);
}
