import sys
path, testfile = sys.argv[1], sys.argv[2]
s = open(path).read()
t = open(testfile).read()
i = s.index('mod tests {')
j = s.index('    #[test]', i)
s = s[:j] + t + '\n' + s[j:]
open(path, 'w').write(s)
