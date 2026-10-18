// Near miss: a production `test`/`it`-shaped call is not a test.
const re = /a/;
function check(s) { return re.test(s); }
function onEach(cb) { return [1].forEach(cb); }
module.exports = { check, onEach };
