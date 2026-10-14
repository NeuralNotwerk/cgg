from scripts import Script
def test_a():
    class TestScript(Script):
        def as_form(self):
            pass
    TestScript().as_form()
