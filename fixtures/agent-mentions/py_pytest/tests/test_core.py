import pytest
from pkg.core import add, Thing


@pytest.fixture
def local_fixture():
    return 1


@pytest.fixture(params=[1, 2])
def param_fixture(request):
    return request.param


def provider_cases():
    return [(1, 2), (3, 4)]


def test_add(shared_fixture, local_fixture, param_fixture):
    assert add(shared_fixture, local_fixture) == 42


@pytest.mark.usefixtures("usefix_fixture")
def test_thing():
    t = Thing()
    t.value = 3
    assert t.value == 3


@pytest.fixture
def usefix_fixture():
    yield


def _test_helper_dead():
    return 0
