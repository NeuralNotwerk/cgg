class Repo { quux() { return 1; } }
class Other { quux() { return 2; } }
export class Svc {
  constructor() { this.repo = new Repo(); }
  syncM() { this.repo.quux(); }
  async asyncM() { this.repo.quux(); }
  syncLocal() { const r = new Repo(); r.quux(); }
  async asyncLocal() { const r = new Repo(); r.quux(); }
}
