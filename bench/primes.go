// primes.go — trial-division primality test below 100,000.
// Prints "9592 454396537" (count, sum).
package main

import (
	"fmt"
	"math"
)

func isPrime(n float64) bool {
	if n < 2.0 {
		return false
	}
	for d := 2.0; d*d <= n; d += 1.0 {
		if math.Mod(n, d) == 0 {
			return false
		}
	}
	return true
}

func main() {
	var count, sum uint64
	for i := 2.0; i < 100000.0; i += 1.0 {
		if isPrime(i) {
			count++
			sum += uint64(i)
		}
	}
	fmt.Printf("%d %d\n", count, sum)
}
