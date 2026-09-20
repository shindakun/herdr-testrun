"""Fixture module with one deliberate bug."""


def add(a, b):
    return a + b


def double(n):
    # Wrong on purpose.
    return n + n + 1
