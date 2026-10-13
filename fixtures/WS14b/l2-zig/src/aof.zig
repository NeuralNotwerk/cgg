pub fn run() !void {
    const IO = @import("io.zig").IO;
    var io = try IO.init(32);
    defer io.deinit();
}
