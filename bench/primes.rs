// primes.rs — trial-division primality test below 100,000, using the same
// floating-point fmod semantics as every other language here (`%` on f64).
// Prints "9592 454396537" (count, sum).
fn is_prime(n: f64) -> bool {
    if n < 2.0 {
        return false;
    }
    let mut d = 2.0;
    while d * d <= n {
        if n % d == 0.0 {
            return false;
        }
        d += 1.0;
    }
    true
}

fn main() {
    let mut count: u64 = 0;
    let mut sum: u64 = 0;
    let mut i = 2.0;
    while i < 100000.0 {
        if is_prime(i) {
            count += 1;
            sum += i as u64;
        }
        i += 1.0;
    }
    println!("{} {}", count, sum);
}
