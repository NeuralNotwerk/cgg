package com.ex;

import org.testng.annotations.DataProvider;
import org.testng.annotations.Test;

public class NgTest {
    @DataProvider(name = "ngData")
    private Object[][] ngDataMethod() { return new Object[][] {{1}}; }

    private Object[][] ngUnannotatedProvider() { return new Object[][] {{2}}; }

    @Test(dataProvider = "ngData")
    public void ngUses(int x) { }

    @Test(dataProvider = "ngUnannotatedProvider")
    public void ngUses2(int x) { }
}
