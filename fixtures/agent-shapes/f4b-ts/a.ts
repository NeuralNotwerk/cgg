class Repo { quux(): number { return 1; } }
class Other { quux(): number { return 2; } }
export class A {
  repo: Repo = new Repo();
  ctlField() { this.repo.quux(); }
  multiRecv() {
    this
      .repo
      .quux();
  }
}
