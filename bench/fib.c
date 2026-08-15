/* fib.c — recursion and conditionals. Prints 832040 for fib(30). */
#include <stdio.h>

static double fib(double n) {
    if (n < 2.0) return n;
    return fib(n - 1.0) + fib(n - 2.0);
}

int main(void) {
    printf("%.0f\n", fib(30));
    return 0;
}
