// The page functions page (ADR 0085): a global of the page's own, which a
// page function sees in the main world and not in an isolated one, and what
// the page's own first script found of an init script's mark, which an init
// script that ran before it set. The page also serves as a frame of itself.
window.pageGlobal = "from the page";
window.initScriptSeenByPage = window.initScriptMark === undefined ? null : window.initScriptMark;
