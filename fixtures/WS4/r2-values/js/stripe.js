const welcome = require('./welcome');
const svc = require('./plain');
function charge() {
  welcome.api.isActive();
  svc.run();
  svc.missing();
}
module.exports = { charge };
