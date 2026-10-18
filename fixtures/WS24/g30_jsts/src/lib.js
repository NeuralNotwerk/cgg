function usedOnlyByTests() { return deep(); }
function deep() { return 1; }
function trulyDead() { return 2; }
function usedInProd() { return 3; }
module.exports = { usedOnlyByTests, usedInProd, trulyDead };
