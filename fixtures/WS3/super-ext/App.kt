package com.acme.app
import android.app.Application
class Other { fun onCreate() { } }
class Third { fun onCreate() { } }
class MyApp : Application() {
    override fun onCreate() { super.onCreate() }   // base external: refuse
}
open class Base { open fun start() { } }
class Mid : Base()
class Leaf : Mid() { override fun start() { super.start() } }  // inherited: Base.start
