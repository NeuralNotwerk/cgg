class Service {
public:
  void run() {}
};
void main2() {
  auto service = mk();
  service.run();
}
