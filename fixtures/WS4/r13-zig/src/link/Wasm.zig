const GlobalImportId = struct {
    pub fn pack(x: u32) u32 {
        return x;
    }
    pub fn unpack(x: u32) u32 {
        return pack(x);
    }
};
const Other = struct {
    pub fn pack(x: u32) u32 {
        return x + 2;
    }
};
