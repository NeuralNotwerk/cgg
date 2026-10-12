class Repo { quux(): number { return 1; } }
class Other { quux(): number { return 2; } }
export class Svc {
  constructor(private repo: Repo) {}
  syncM() { this.repo.quux(); }
  async asyncM() { this.repo.quux(); }
  syncLocal() { const r: Repo = this.repo; r.quux(); }
  async asyncLocal() { const r: Repo = this.repo; r.quux(); }
  syncParam(r: Repo) { r.quux(); }
  async asyncParam(r: Repo) { r.quux(); }
}
export const arrowSync = (r: Repo) => { r.quux(); };
export const arrowAsync = async (r: Repo) => { r.quux(); };
