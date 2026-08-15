// loop.go — 2,700,000-iteration counter loop. Prints 3644998650000.
package main

import "fmt"

func main() {
	var n, total float64
	for n < 2700000.0 {
		total += n
		n += 1.0
	}
	fmt.Printf("%.0f\n", total)
}
