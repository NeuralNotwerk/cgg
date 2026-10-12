function mapTarget(x) { return x + 1; }
function clickHandler(e) { console.log(e); }
function timeoutTarget() { console.log("t"); }
function exportedHandler() { return 1; }
function promiseThen(v) { return v; }
function assignedFn() { return 2; }
function returnedFn() { return 3; }
function arrayMember() { return 4; }
function deadControl() { return 0; }
function calledControl() { return 5; }

class Widget {
  #privDead() { return 0; }
  #privMapTarget(x) { return x; }
  render() { return [1].map(this.#privMapTarget); }
  onClick() { return 9; }
  bind() { document.body.addEventListener("click", this.onClick); }
}

function makeHandler() { return returnedFn; }

function main() {
  const xs = [1, 2, 3].map(mapTarget);
  document.body.addEventListener("click", clickHandler);
  setTimeout(timeoutTarget, 10);
  Promise.resolve(1).then(promiseThen);
  const h = assignedFn;
  const list = [arrayMember];
  calledControl();
  new Widget().render();
  new Widget().bind();
  return [xs, h, list, makeHandler()];
}

main();

export default { handler: exportedHandler };
