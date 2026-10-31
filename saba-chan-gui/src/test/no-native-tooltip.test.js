/**
 * 마우스를 올렸을 때 OS 기본 툴팁(title 속성, SVG <title>)이 뜨지 않게 한다.
 * 설명 글은 `data-tooltip`으로 달아 앱 말풍선(TooltipProvider)이 그린다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { parse } from '@babel/parser';
import { describe, expect, it } from 'vitest';

const SRC = path.resolve(__dirname, '..');

function sourceFiles(dir) {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' || e.name === 'dev' ? [] : sourceFiles(p);
        return /\.jsx?$/.test(e.name) && !e.name.includes('.test.') ? [p] : [];
    });
}

/** 소문자 태그(DOM·SVG 요소)에 붙은 title 속성과 <title> 요소의 줄 번호 */
function nativeTooltips(code) {
    const ast = parse(code, { sourceType: 'module', plugins: ['jsx'] });
    const found = [];
    const walk = (node) => {
        if (!node || typeof node.type !== 'string') return;
        if (node.type === 'JSXOpeningElement' && node.name.type === 'JSXIdentifier' && /^[a-z]/.test(node.name.name)) {
            if (node.name.name === 'title') found.push(node.loc.start.line);
            for (const a of node.attributes) {
                if (a.type === 'JSXAttribute' && a.name.name === 'title') found.push(a.loc.start.line);
            }
        }
        for (const v of Object.values(node)) {
            if (Array.isArray(v)) v.forEach(walk);
            else if (v && typeof v === 'object' && v.type) walk(v);
        }
    };
    walk(ast.program);
    return found;
}

describe('OS 기본 툴팁', () => {
    it('검사기는 DOM 요소의 title과 SVG <title>만 찾고 컴포넌트 title prop은 넘긴다', () => {
        const code = [
            '<Modal title="a" />',
            '<button title="b" />',
            '<svg><title>c</title></svg>',
            '<span data-tooltip="d" />',
        ].join(';\n');
        expect(nativeTooltips(code)).toEqual([2, 3]);
    });

    it('GUI 코드에 OS 기본 툴팁이 없다', () => {
        const found = [];
        for (const file of sourceFiles(SRC)) {
            for (const line of nativeTooltips(fs.readFileSync(file, 'utf8'))) {
                found.push(`${path.relative(SRC, file)}:${line}`);
            }
        }
        expect(found).toEqual([]);
    });
});
