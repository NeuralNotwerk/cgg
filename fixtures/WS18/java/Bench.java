package p;
import org.openjdk.jmh.annotations.Benchmark;
import org.openjdk.jmh.annotations.Setup;
import io.cucumber.java.en.Given;
import javax.persistence.Column;
public class Bench {
  @Benchmark public void run() {}
  @Setup public void prep() {}
  @Given("a thing") public void aThing() {}
  @Column(name="x") public String getX() { return ""; }
  private void deadOne() {}
}
