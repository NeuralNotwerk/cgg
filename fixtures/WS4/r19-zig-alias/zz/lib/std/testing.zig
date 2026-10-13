pub fn expect(ok: bool) !void {
    if (!ok) return error.TestUnexpectedResult;
}
