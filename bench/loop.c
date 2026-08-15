/* loop.c — 2,700,000-iteration counter loop. Prints 3644998650000. */
#include <stdio.h>

int main(void) {
    double n = 0.0, total = 0.0;
    while (n < 2700000.0) {
        total += n;
        n += 1.0;
    }
    printf("%.0f\n", total);
    return 0;
}
