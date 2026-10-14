package demo;

import org.testng.annotations.DataProvider;
import org.testng.annotations.Test;

class NgTest {
    @DataProvider(name = "rows")
    private Object[][] makeRows() { return new Object[][] {{1}}; }

    @DataProvider
    private Object[][] plain() { return new Object[][] {{2}}; }

    @Test(dataProvider = "rows")
    public void usesRows(int x) {}

    // near miss
    private Object[][] notAProvider() { return null; }
}
