const std = @import("std");
// std is lib/std/std.zig here: follow the alias to lib/std/testing.zig.
const expect = std.testing.expect;

pub fn checkIt(ok: bool) !void {
    try expect(ok);
}
