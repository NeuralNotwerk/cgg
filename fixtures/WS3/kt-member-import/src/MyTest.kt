package org.acme.t
import app.cash.turbine.test
import org.acme.BaseUnitTest
class MyTest : BaseUnitTest() {
    fun check() { test { } }      // the inherited member, not Turbine's import nor org.acme.other.test
}
