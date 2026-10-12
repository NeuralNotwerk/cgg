class FileSystem { void directory(String p) {} }
class Fake { void directory(String p) {} }
void make(int x, {FileSystem? fs}) { fs.directory('/'); }
