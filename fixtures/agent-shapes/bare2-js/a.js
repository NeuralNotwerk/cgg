class Repo {
  zlist(d) { return d; }
  async azlist(d) { return d; }
  static szlist(d) { return d; }
  caller(data) { zlist(data); azlist(data); szlist(data); }
}
const repo = {
  olist(d) { return d; },
  async aolist(d) { return d; },
};
function top(data) { zlist(data); azlist(data); olist(data); aolist(data); }
