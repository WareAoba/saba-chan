"""콘솔 글꼴 줄이기: 원본 글꼴에서 콘솔이 쓰는 글자 범위만 남겨 woff2로 (src/assets/fonts)

    python -I scripts/subset-console-fonts.py <원본 폴더> src/assets/fonts

범위는 src/styles/consoleFonts.css 의 unicode-range 와 같다. 영문 칸 + 그 글꼴이 맡는 글자 칸만 남긴다.
원본 파일 이름과 받은 곳은 src/assets/fonts/LICENSES.md. Cascadia Code는 Reserved Font Name이 있어 줄이지 않고
공식 woff2를 그대로 쓴다. 가변 글꼴(M PLUS 1 Code)은 굵기 400 한 벌로. fontTools·brotli 필요
"""
import sys, os
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
src, out = sys.argv[1], sys.argv[2]
R = {
 'latin': 'U+0000-024F,U+0370-04FF,U+1E00-1EFF,U+2000-22FF,U+2500-259F',
 'ko': 'U+1100-11FF,U+3130-318F,U+A960-A97F,U+AC00-D7AF,U+D7B0-D7FF',
 'ja': 'U+3000-303F,U+3040-30FF,U+31F0-31FF,U+4E00-9FFF,U+FF00-FFEF',
 'zh': 'U+3000-303F,U+3400-4DBF,U+4E00-9FFF,U+FF00-FFEF',
}
FONTS = {
 'd2coding': ('D2Coding.woff2', ['latin','ko']),
 'nanum-gothic-coding': ('NanumGothicCoding-Regular.ttf', ['latin','ko']),
 'neodgm-code': ('neodgm_code.woff2', ['latin','ko']),
 'sarasa-mono-j': ('SarasaMonoJ-Regular.ttf', ['latin','ja']),
 'udev-gothic': ('UDEVGothic-Regular.ttf', ['latin','ja']),
 'hackgen': ('HackGen-Regular.ttf', ['latin','ja']),
 'm-plus-1-code': ('MPLUS1Code[wght].ttf', ['latin','ja']),
 'sarasa-mono-sc': ('SarasaMonoSC-Regular.ttf', ['latin','zh']),
 'sarasa-mono-tc': ('SarasaMonoTC-Regular.ttf', ['latin','zh']),
}
def parse(r):
    cps=set()
    for part in r.split(','):
        a,_,b = part[2:].partition('-'); a=int(a,16); b=int(b or part[2:],16)
        cps.update(range(a,b+1))
    return cps
for fid,(fn,slots) in FONTS.items():
    f = TTFont(os.path.join(src,fn))
    if 'fvar' in f:
        f = instancer.instantiateVariableFont(f, {'wght':400})
    cps=set().union(*(parse(R[s]) for s in slots))
    opts = subset.Options(); opts.flavor='woff2'; opts.layout_features=['*']; opts.name_IDs=['*']; opts.notdef_outline=True
    opts.hinting=False
    s = subset.Subsetter(opts); s.populate(unicodes=cps); s.subset(f)
    p=os.path.join(out,fid+'.woff2'); f.flavor='woff2'; f.save(p)
    print(fid, round(os.path.getsize(p)/1e6,2),'MB')
