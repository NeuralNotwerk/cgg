using System.IO;
using Newtonsoft.Json;
namespace Acme {
  public class App {
    public void Run() {
      var a = Path.Combine("a", "b");
      var t = File.ReadAllText("x");
      var j = JsonConvert.SerializeObject(this);
      var k = System.IO.Path.GetTempPath();
      var m = Newtonsoft.Json.JsonConvert.SerializeObject(this);
    }
  }
}
