// fib.go — recursion and conditionals. Prints 832040 for fib(30).
package main

import "fmt"

func fib(n float64) float64 {
	if n < 2.0 {
		return n
	}
	return fib(n-1.0) + fib(n-2.0)
}

func main() {
	fmt.Printf("%.0f\n", fib(30))
}
