# fib.py — recursion and conditionals. Prints 832040 for fib(30).
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)


print(f"{fib(30):.0f}")
