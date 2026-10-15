package task;
import util.ISE;
public class Task {
  void run(int n) {
    throw new ISE("bad %s", n);
  }
}
