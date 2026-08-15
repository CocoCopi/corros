/* primes.c — trial-division primality test below 100,000, using the same
 * floating-point fmod semantics as every other language here.
 * Prints "9592 454396537" (count, sum). */
#include <stdio.h>
#include <math.h>

static int is_prime(double n) {
    if (n < 2.0) return 0;
    for (double d = 2.0; d * d <= n; d += 1.0)
        if (fmod(n, d) == 0.0) return 0;
    return 1;
}

int main(void) {
    long long count = 0, sum = 0;
    for (double i = 2.0; i < 100000.0; i += 1.0)
        if (is_prime(i)) {
            count++;
            sum += (long long)i;
        }
    printf("%lld %lld\n", count, sum);
    return 0;
}
