package app

import com.ext.Base

class MainAct : Base() {
    override fun hookK() { helper() }
    private fun helper() {}
    private fun unusedK() {}
}
