# 把 dist/index.html 轉成 Artifact 用的頁面片段（去掉 doctype/html/head/body 外殼）
import re, pathlib
src = pathlib.Path(__file__).parent / 'dist' / 'index.html'
s = src.read_text()
head = re.search(r'<head>(.*?)</head>', s, re.S).group(1)
body = re.search(r'<body[^>]*>(.*)</body>', s, re.S).group(1)
head = re.sub(r'<meta[^>]*>\s*', '', head)
# title 放最前面
title = re.search(r'<title>.*?</title>', head).group(0)
head = head.replace(title, '')
out = title + '\n' + head + '\n' + body
(pathlib.Path(__file__).parent / 'dist' / 'artifact.html').write_text(out)
print('artifact.html', len(out))
