import pytest


@pytest.fixture
def shared_fixture():
    return 41


@pytest.fixture
def _private_fixture():
    return 1


def _conftest_dead_helper():
    return 2
