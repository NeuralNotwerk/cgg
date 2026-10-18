const { usedOnlyByTests } = require('../src/lib');
function makeFixture() { return { a: 1 }; }
function unusedTestHelper() { return 0; }
describe('lib', () => {
  beforeEach(() => { makeFixture(); });
  it('works', () => {
    expect(usedOnlyByTests()).toBe(1);
  });
  test('named callback', namedCase);
});
function namedCase() { makeFixture(); }
