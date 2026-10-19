const std = @import("std");
const mod = @import("mod.zig");
const Self = @This();
const VTable = struct { alloc: *const fn () void, free: *const fn () void };
pub const vtable = VTable{ .alloc = alloc, .free = mod.free };
fn alloc() void {}
fn lessThan(a: u8, b: u8) bool { return a < b; }
fn worker() void {}
fn stored() void {}
fn returned() void {}
fn viaSelf() void {}
fn orphan() void {}
fn shadowed() void {}
pub fn main() void {
    var xs = [_]u8{ 3, 1 };
    std.sort.block(u8, &xs, {}, lessThan);
    _ = std.Thread.spawn(.{}, worker, .{}) catch {};
    var cb: *const fn () void = &stored;
    cb = &Self.viaSelf;
    _ = pick();
    use(1);
    const t = .{elsewhere};
    _ = t;
}
fn pick() *const fn () void { return returned; }
fn use(shadowed: u8) void { _ = shadowed; }
