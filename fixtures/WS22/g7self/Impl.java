package s.impl;
import m.Mapper;
import s.Service;
public class Impl implements Service {
  @Autowired private Mapper mapper;
  @Override public void del(Long id, String u) { mapper.del(id, u); }
}
