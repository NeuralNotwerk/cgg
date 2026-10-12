package com.acme.app

import com.acme.model.Base
import com.acme.model.Widget
import com.acme.model.Registry
import java.io.File

class Screen(val w: Widget) : Base() {
    fun close() { }
    fun paint() { }
    override fun greet() {
        super.greet()            // base Base.greet, not Screen.greet
        this.paint()             // own Screen.paint
        Widget().paint()         // constructed: Widget.paint
        com.acme.model.Registry.lookup("a")   // FQN object call
        Registry.lookup("b").paint()          // call-result receiver: unknown -> not Screen.paint
        w.paint()                // param/property typed (control)
        File("x").readText()     // external constructed type
        val o: Screen? = null
        o!!.close()              // typed via local
    }
}
