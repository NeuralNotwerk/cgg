class Repo { int quux() => 1; }
class Other { int quux() => 2; }
void syncParam(Repo r) { r.quux(); }
Future<void> asyncParam(Repo r) async { r.quux(); }
