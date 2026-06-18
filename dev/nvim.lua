local capabilities = vim.lsp.protocol.make_client_capabilities()
capabilities.workspace.workspaceEdit.documentChanges = true

vim.lsp.config['djot-language-server'] = {
  cmd = { './target/debug/djot-ls' },
  filetypes = { 'djot' },
  root_dir = vim.fs.root(0, { '.git' }),
  capabilities = capabilities,
}

vim.lsp.enable('djot-language-server')
