const legacy = require('./legacy');
const proxy = require('./proxy');
const fn = require('./fnmod');
const Store = require('./klass');
function go() {
  legacy.run();
  legacy.other();
  proxy.run();
  fn();
  Store.open();
}
module.exports = { go };
