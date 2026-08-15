// fib.rs — recursion and conditionals. Prints 832040 for fib(30).
fn fib(n: f64) -> f64 {
    if n < 2.0 {
        n
    } else {
        fib(n - 1.0) + fib(n - 2.0)
    }
}

fn main() {
    println!("{:.0}", fib(30.0));
}
