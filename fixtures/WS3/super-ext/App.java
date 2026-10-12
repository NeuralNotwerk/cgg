package com.acme.j;
import android.app.Activity;
class Other { void onResume() { } }
class Third { void onResume() { } }
class MyActivity extends Activity {
    @Override protected void onResume() { super.onResume(); }
}
