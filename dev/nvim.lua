vim.lsp.config['djot-language-server'] = {
  cmd = { './target/debug/djot-ls' },
  filetypes = { 'djot' },
}

vim.lsp.enable('djot-language-server')
