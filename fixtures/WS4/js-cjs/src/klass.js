class Store {
  static open() { return new Store(); }
  get() { return 1; }
}
module.exports = Store;
