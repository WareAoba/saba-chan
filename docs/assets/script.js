/* ═══════════════════════════════════════════════════════════
   사바쨩 문서 — UX 스크립트
   테마 토글 · 앵커 생성 · 사이드바 하이라이트 · 목차 접기
   ═══════════════════════════════════════════════════════════ */

(() => {
    // ── Theme ──
    const THEME_KEY = "saba-docs-theme";
    const root = document.documentElement;

    function applyTheme(theme) {
        if (theme === "light" || theme === "dark") {
            root.setAttribute("data-theme", theme);
        } else {
            const prefersDark = window.matchMedia(
                "(prefers-color-scheme: dark)"
            ).matches;
            root.setAttribute("data-theme", prefersDark ? "dark" : "light");
        }
    }

    const saved = localStorage.getItem(THEME_KEY);
    applyTheme(saved || "auto");

    document.addEventListener("click", (e) => {
        const btn = e.target.closest("[data-theme-toggle]");
        if (!btn) return;
        const current = root.getAttribute("data-theme") || "light";
        const next = current === "dark" ? "light" : "dark";
        applyTheme(next);
        localStorage.setItem(THEME_KEY, next);
    });

    // ── Year stamp ──
    document.querySelectorAll("[data-year]").forEach((el) => {
        el.textContent = new Date().getFullYear();
    });

    // ── Auto-generate anchors for h2/h3/h4 in .content ──
    const slugify = (text) =>
        text
            .toLowerCase()
            .replace(/[^\p{L}\p{N}\s-]/gu, "")
            .trim()
            .replace(/\s+/g, "-");

    const content = document.querySelector(".content");
    if (content) {
        content.querySelectorAll("h2, h3, h4").forEach((h) => {
            if (!h.id) {
                const base = slugify(h.textContent || "");
                if (base) h.id = base;
            }
            if (h.id && !h.querySelector(".anchor")) {
                const a = document.createElement("a");
                a.className = "anchor";
                a.href = `#${h.id}`;
                a.setAttribute("aria-label", "해당 섹션 링크");
                a.textContent = "#";
                h.appendChild(a);
            }
        });
    }

    // ── Sidebar active highlight (scroll spy) ──
    const sidebarLinks = Array.from(
        document.querySelectorAll(".sidebar-list a[href^='#']")
    );
    if (sidebarLinks.length && content) {
        const idToLink = new Map();
        sidebarLinks.forEach((a) => {
            const id = decodeURIComponent(a.getAttribute("href").slice(1));
            idToLink.set(id, a);
        });

        const observer = new IntersectionObserver(
            (entries) => {
                entries.forEach((entry) => {
                    if (entry.isIntersecting) {
                        const id = entry.target.id;
                        sidebarLinks.forEach((a) => a.classList.remove("active"));
                        const link = idToLink.get(id);
                        if (link) {
                            link.classList.add("active");
                            const parentLi = link.closest(".sidebar-sub")?.parentElement;
                            if (parentLi) {
                                const topLink = parentLi.querySelector(":scope > a");
                                if (topLink) topLink.classList.add("active");
                            }
                        }
                    }
                });
            },
            {
                rootMargin: "-40% 0px -50% 0px",
                threshold: 0,
            }
        );

        idToLink.forEach((_, id) => {
            const target = document.getElementById(id);
            if (target) observer.observe(target);
        });
    }

    // ── Wrap tables for horizontal scroll ──
    if (content) {
        content.querySelectorAll("table").forEach((table) => {
            if (table.parentElement?.classList.contains("table-wrap")) return;
            const wrap = document.createElement("div");
            wrap.className = "table-wrap";
            table.parentNode.insertBefore(wrap, table);
            wrap.appendChild(table);
        });
    }

    // ── Mobile sidebar toggle ──
    const sidebar = document.querySelector(".sidebar");
    if (sidebar && window.matchMedia("(max-width: 960px)").matches) {
        sidebar.classList.add("collapsed");
    }
    document.addEventListener("click", (e) => {
        const btn = e.target.closest(".sidebar-toggle");
        if (!btn || !sidebar) return;
        sidebar.classList.toggle("collapsed");
    });
})();
