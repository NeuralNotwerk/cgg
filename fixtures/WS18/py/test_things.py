import unittest
import pytest
class TestA(unittest.TestCase):
    def setUp(self): pass
    def test_one(self): self.helper()
    def helper(self): pass
    def testing_is_not_a_test(self): pass
def test_free(): pass
@pytest.fixture
def db(): pass
