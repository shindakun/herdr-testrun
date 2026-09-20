import pytest

from mathx import add, double


def test_add():
    assert add(1, 2) == 3


def test_double():
    assert double(2) == 4


@pytest.fixture
def broken_fixture():
    raise RuntimeError("fixture setup fails on purpose")


def test_uses_broken_fixture(broken_fixture):
    assert True
