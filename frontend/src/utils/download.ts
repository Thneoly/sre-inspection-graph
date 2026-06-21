/**
 * 浏览器端触发文件下载 — PRD-003 Sprint 1。
 *
 * 项目里首个 blob 下载场景(报告 .md)。用 URL.createObjectURL + 临时 <a download> click。
 * 用完即 revoke,避免内存泄漏。
 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  // 释放对象 URL(下一次事件循环,确保 click 已派发)
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
