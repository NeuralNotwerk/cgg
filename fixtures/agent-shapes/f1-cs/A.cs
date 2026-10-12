using System.Threading.Tasks;
namespace P {
class Repo { public int Quux() { return 1; } }
class Other { public int Quux() { return 2; } }
public class Svc {
    void SyncParam(Repo r) { r.Quux(); }
    async Task AsyncParam(Repo r) { r.Quux(); }
    void SyncLocal() { var r = new Repo(); r.Quux(); }
    async Task AsyncLocal() { var r = new Repo(); r.Quux(); }
    async Task<int> AsyncRet(Repo r) { return r.Quux(); }
}
}
