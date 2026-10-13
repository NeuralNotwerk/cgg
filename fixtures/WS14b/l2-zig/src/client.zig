const Header = @import("header.zig").Header;

fn other_frame_const() void {}

pub fn send(p: *const Header.Ping) u32 {
    return p.frame_const();
}
