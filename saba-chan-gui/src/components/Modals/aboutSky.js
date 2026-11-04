/**
 * 정보 페이지 제작자 카드의 밤하늘: 별(2D 캔버스)과 오로라(WebGL).
 * 둘 다 30fps 로 묶고, 화면 밖에 있거나 창이 가려지면 멈춘다. 움직임 줄이기 설정이면 한 장만 그린다.
 * start* 는 멈추개를 돌려준다. 그릴 수 없는 환경(캔버스·WebGL 없음)이면 아무것도 하지 않는다.
 */

const FRAME_MS = 33;

const prefersReducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches === true;

/** 보일 때만 draw(t) 를 30fps 로 부르는 고리. draw 는 처음 한 번은 꼭 불린다 */
function loop(el, draw) {
    const still = prefersReducedMotion();
    let visible = true;
    let prev = 0;
    let raf = 0;
    let stopped = false;
    const io = typeof IntersectionObserver === 'function'
        ? new IntersectionObserver(([e]) => { visible = e.isIntersecting; })
        : null;
    io?.observe(el);
    const tick = (t) => {
        if (stopped) return;
        if (!prev || (visible && !document.hidden && t - prev >= FRAME_MS)) {
            prev = t || 1;
            draw(still ? 40000 : t);
        }
        if (!still) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return {
        redraw() { prev = 0; if (still) raf = requestAnimationFrame(tick); },
        stop() { stopped = true; cancelAnimationFrame(raf); io?.disconnect(); },
    };
}

/** 같은 자리에 같은 별이 뜨도록 씨앗이 고정된 난수 */
function seeded(seed) {
    let s = seed;
    return () => (s = (s * 16807) % 2147483647) / 2147483647;
}

/** 별: 작은 별은 한 번만 그려 두고, 큰 별만 반짝인다. 가끔 별똥별이 지나간다 */
export function startStars(canvas) {
    const ctx = canvas.getContext?.('2d');
    if (!ctx) return () => {};
    const still = prefersReducedMotion();
    const base = document.createElement('canvas');
    let W = 0;
    let H = 0;
    let big = [];
    let shoot = null;
    let lastShoot = 0;

    const fit = () => {
        const r = canvas.getBoundingClientRect();
        const d = window.devicePixelRatio || 1;
        W = r.width;
        H = r.height;
        canvas.width = base.width = Math.max(1, Math.round(W * d));
        canvas.height = base.height = Math.max(1, Math.round(H * d));
        ctx.setTransform(d, 0, 0, d, 0, 0);
        const b = base.getContext('2d');
        if (!b) return;
        b.setTransform(d, 0, 0, d, 0, 0);
        const rnd = seeded(7);
        big = [];
        for (let i = Math.round((W * H) / 800); i > 0; i--) {
            const s = { x: rnd() * W, y: rnd() * H * 0.9, r: rnd() < 0.07 ? 1.4 + rnd() : 0.35 + rnd() * 0.8,
                p: rnd() * 6.28, sp: 0.6 + rnd() * 1.6, hue: rnd() < 0.25 ? 260 : 210 };
            if (s.r > 1.4) { big.push(s); continue; }
            b.fillStyle = `hsla(${s.hue},100%,92%,${0.35 + 0.5 * rnd()})`;
            b.beginPath();
            b.arc(s.x, s.y, s.r, 0, 6.28);
            b.fill();
        }
    };

    const draw = (t) => {
        ctx.clearRect(0, 0, W, H);
        ctx.drawImage(base, 0, 0, W, H);
        for (const s of big) {
            const a = still ? 0.8 : 0.45 + 0.55 * Math.abs(Math.sin(s.p + (t / 1000) * s.sp));
            ctx.fillStyle = `hsla(${s.hue},100%,92%,${a})`;
            ctx.beginPath();
            ctx.arc(s.x, s.y, s.r, 0, 6.28);
            ctx.fill();
            ctx.strokeStyle = `hsla(${s.hue},100%,92%,${a * 0.5})`;
            ctx.lineWidth = 0.6;
            ctx.beginPath();
            ctx.moveTo(s.x - 4, s.y); ctx.lineTo(s.x + 4, s.y);
            ctx.moveTo(s.x, s.y - 4); ctx.lineTo(s.x, s.y + 4);
            ctx.stroke();
        }
        if (still) return;
        if (!shoot && t - lastShoot > 5200) {
            shoot = { x: W * (0.35 + Math.random() * 0.6), y: H * Math.random() * 0.25, k: 0 };
            lastShoot = t;
        }
        if (shoot) {
            shoot.k += 0.04;
            const len = 70;
            const x = shoot.x - shoot.k * 180;
            const y = shoot.y + shoot.k * 90;
            const g = ctx.createLinearGradient(x, y, x + len, y - len / 2);
            g.addColorStop(0, `rgba(255,255,255,${Math.max(0, 1 - shoot.k)})`);
            g.addColorStop(1, 'rgba(255,255,255,0)');
            ctx.strokeStyle = g;
            ctx.lineWidth = 1.4;
            ctx.beginPath();
            ctx.moveTo(x, y);
            ctx.lineTo(x + len, y - len / 2);
            ctx.stroke();
            if (shoot.k >= 1) shoot = null;
        }
    };

    fit();
    const l = loop(canvas, draw);
    const onResize = () => { fit(); l.redraw(); };
    window.addEventListener('resize', onResize);
    return () => { l.stop(); window.removeEventListener('resize', onResize); };
}

/*
 * 오로라 셰이더. 실제 오로라처럼:
 *  - 커튼 아래 가장자리가 가장 밝고 날카로운 초록(산소 557.7nm), 위로 갈수록 옅어지고 높은 곳은 붉은 보라(산소 630nm·질소)
 *  - 커튼은 노이즈로 굽이치고, 세로 빛살(자기력선 방향)이 굽이를 따라 휘며 옆으로 흐르고 깜빡인다
 *  - 지평선 가까이는 대기에 묻혀 옅다
 */
const VERTEX = 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}';
const FRAGMENT = `precision mediump float;
uniform vec2 R; uniform float T;
float h(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
float n(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);
  return mix(mix(h(i),h(i+vec2(1,0)),f.x),mix(h(i+vec2(0,1)),h(i+vec2(1,1)),f.x),f.y);}
float fbm(vec2 p){float v=0.,a=.5;for(int i=0;i<4;i++){v+=a*n(p);p=p*2.03+vec2(1.7,9.2);a*=.5;}return v;}
vec3 curtain(vec2 uv,float o,float base,float amp,float bright){
  float x=uv.x;
  float w=fbm(vec2(x*.9+o,T*.035+o));
  float y0=base+amp*(w-.5)+.05*sin(x*2.3+T*.12+o*3.);
  float d=uv.y-y0;
  float body=d<0.?exp(-d*d*1400.):exp(-d*2.6)*(1.-smoothstep(.55,1.1,d));
  float rx=x*22.+w*9.-T*.22+o*7.;
  float rays=n(vec2(rx,d*1.2-T*.15))*.65+n(vec2(rx*2.7,d*.8+T*.1))*.35;
  rays=.25+.75*smoothstep(.25,.95,rays);
  float flick=.7+.3*n(vec2(x*5.+o,T*.9));
  vec3 c=mix(vec3(.18,1.,.45),vec3(.15,.85,.75),smoothstep(.02,.18,d));
  c=mix(c,vec3(.75,.28,.7),smoothstep(.16,.5,d));
  c+=vec3(.55,.6,.45)*exp(-abs(d)*90.)*.7;
  return c*body*rays*flick*bright;
}
void main(){
  vec2 uv=gl_FragCoord.xy/R.y; uv.y-=.02;
  float ar=R.x/R.y;
  vec3 c=curtain(uv,0.,.42,.42,1.)+curtain(uv,4.3,.62,.35,.55)+curtain(uv,9.1,.30,.30,.35);
  c*=smoothstep(0.,.18,uv.y);
  c*=.55+.45*smoothstep(0.,.35*ar,uv.x)*smoothstep(ar,.65*ar,uv.x)+.2;
  c=(1.-exp(-c*1.35))*.62;
  gl_FragColor=vec4(c,max(c.g,max(c.r,c.b)));
}`;

/** 오로라를 반 해상도로 그린다 (늘릴 때 저절로 번져 부드럽다) */
export function startAurora(canvas) {
    const gl = canvas.getContext?.('webgl', { premultipliedAlpha: true, alpha: true, antialias: false });
    if (!gl) return () => {};
    const compile = (type, src) => {
        const s = gl.createShader(type);
        gl.shaderSource(s, src);
        gl.compileShader(s);
        return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
    };
    const vs = compile(gl.VERTEX_SHADER, VERTEX);
    const fs = compile(gl.FRAGMENT_SHADER, FRAGMENT);
    if (!vs || !fs) return () => {};
    const prog = gl.createProgram();
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return () => {};
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, 'p');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const uR = gl.getUniformLocation(prog, 'R');
    const uT = gl.getUniformLocation(prog, 'T');

    const fit = () => {
        const r = canvas.getBoundingClientRect();
        canvas.width = Math.max(1, Math.round(r.width * 0.5));
        canvas.height = Math.max(1, Math.round(r.height * 0.5));
        gl.viewport(0, 0, canvas.width, canvas.height);
        gl.uniform2f(uR, canvas.width, canvas.height);
    };
    const draw = (t) => {
        gl.uniform1f(uT, t / 1000 + 40);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
    };

    fit();
    const l = loop(canvas, draw);
    const onResize = () => { fit(); l.redraw(); };
    window.addEventListener('resize', onResize);
    return () => {
        l.stop();
        window.removeEventListener('resize', onResize);
        // 컨텍스트는 버리지 않는다: 같은 캔버스로 다시 열면 getContext 가 같은 컨텍스트를 돌려준다
        gl.deleteBuffer(buf);
        gl.deleteProgram(prog);
        gl.deleteShader(vs);
        gl.deleteShader(fs);
    };
}
