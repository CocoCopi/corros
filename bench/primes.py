# primes.py — trial-division primality test below 100,000.
# Prints "9592 454396537" (count, sum).
def is_prime(n):
    if n < 2:
        return False
    d = 2
    while d * d <= n:
        if n % d == 0:
            return False
        d += 1
    return True


count = 0
total = 0
i = 2
while i < 100000:
    if is_prime(i):
        count += 1
        total += i
    i += 1

print(count, total)
