// 选项页：本机令牌只能由用户手动粘贴一次——扩展读不到应用的数据目录。

const tokenInput = document.getElementById('token');
const saveButton = document.getElementById('save');
const statusNode = document.getElementById('status');

document.addEventListener('DOMContentLoaded', async () => {
  const stored = await chrome.storage.local.get(['bridgeToken']);
  tokenInput.value = stored.bridgeToken || '';
  saveButton.addEventListener('click', save);
  tokenInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') save();
  });
});

function save() {
  const token = tokenInput.value.trim();
  if (!token) {
    show('令牌不能为空', 'error');
    return;
  }
  chrome.storage.local.set({ bridgeToken: token, bridgePort: null }, () => {
    show('已保存，去点扩展图标试试', 'success');
  });
}

function show(message, kind) {
  statusNode.textContent = message;
  statusNode.className = `status ${kind}`;
  statusNode.hidden = false;
  setTimeout(() => {
    statusNode.hidden = true;
  }, 4000);
}
