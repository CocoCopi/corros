# loop.py — 2,700,000-iteration counter loop. Prints 3644998650000.
n = 0.0
total = 0.0
while n < 2700000.0:
    total += n
    n += 1.0

print(f"{total:.0f}")
