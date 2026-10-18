class Panel extends React.Component {
  state = { open: false };
  handleClick = () => { this.go(); };
  static make = function () { return helper(); };
  #secret = async () => { this.go(); };
  go() { this.handleClick(); this.#secret(); }
  render() { return <button onClick={this.handleClick}>x</button>; }
}
function helper() { return 1; }
