package p
import dagger.Provides
import org.greenrobot.eventbus.Subscribe
class Mod {
  @Provides fun provideX(): Int = 1
  @Subscribe fun onEvent(e: Any) {}
  private fun deadK() {}
}
