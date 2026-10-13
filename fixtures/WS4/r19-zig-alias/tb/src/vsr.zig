const std = @import("std");
// std is not in this tree: the alias is external, never jni_tests' assert.
const assert = std.debug.assert;

pub fn run(x: u32) void {
    assert(x > 0);
}
