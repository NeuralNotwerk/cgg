package m;
public class Fetchers {
  private Daos dao;
  public Object getDatasets() {
    return env -> { return dao.getDatasets(); };
  }
}
