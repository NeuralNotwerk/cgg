package app

import androidx.appcompat.app.AppCompatActivity
import androidx.fragment.app.Fragment
import android.app.Application
import android.os.Bundle

class MainAct : AppCompatActivity() {
    override fun onCreate(b: Bundle?) { setup() }
    private fun setup() {}
    fun notLifecycle() {}
}

class Frag : Fragment() {
    override fun onViewCreated(v: Any, b: Bundle?) {}
}

class App : Application() {
    override fun onCreate() {}
}

// Near miss: not an Activity, same method name.
class Plain {
    fun onCreate() {}
}
