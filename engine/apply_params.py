# 用法：python3 apply_params.py "k=v,..."  把調好的參數寫回 eval.rs 預設值（一般專家 w 與殘局專家 e）
import re, sys
spec = dict(kv.split('=') for kv in sys.argv[1].split(',') if kv)
p = __file__.replace('apply_params.py', 'src/eval.rs')
s = open(p).read()
names = re.search(r'PARAM_NAMES: \[&str; idx::NP\] = \[(.*?)\];', s, re.S).group(1)
names = re.findall(r'"(\w+)"', names)
m = re.search(r'let w = \[(.*?)\];', s, re.S)
body = m.group(1)
# 逐個數值（保留註解）
vals = re.findall(r'-?\d+(?=,)', re.sub(r'//[^\n]*', '', body))
assert len(vals) == len(names), (len(vals), len(names))
lines = body.split('\n')
out = []
k = 0
for line in lines:
    code, _, comment = line.partition('//')
    def rep(mm):
        global k
        name = names[k]; k += 1
        return spec.get(name, mm.group(0))
    code = re.sub(r'-?\d+(?=,)', rep, code)
    out.append(code + ('//' + comment if comment else ''))
s = s[:m.start(1)] + '\n'.join(out) + s[m.end(1):]
# 殘局專家覆寫
eover = {k[2:]: v for k, v in spec.items() if k.startswith('e.')}
m2 = re.search(r'(let mut e = w;\n)(.*?)(        Params \{ w, e \})', s, re.S)
lines2 = ''.join(f'        e[{names.index(n)}] = {v}; // {n}\n' for n, v in eover.items())
if m2:
    s = s[:m2.start(2)] + lines2 + s[m2.end(2):]
else:
    s = s.replace('        Params { w, e: w }', '        #[allow(unused_mut)]\n        let mut e = w;\n' + lines2 + '        Params { w, e }')
open(p, 'w').write(s)
print('ok', k)
