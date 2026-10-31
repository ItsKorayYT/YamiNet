-- Neovim 0.11+ configuration for yami-lsp.
-- Put this file on your runtimepath (for example require it from init.lua) and
-- replace "yami-lsp" with the absolute binary path if it is not on PATH.

vim.filetype.add({
  extension = { ylang = "ylang", ysl = "ysl" },
})

local site_root = vim.fn.getcwd()

vim.lsp.config("yami_lsp", {
  cmd = { "yami-lsp" },
  filetypes = { "ylang", "ysl" },
  root_markers = { "default.ysl", ".git" },
  init_options = {
    siteRoot = site_root,
  },
})

vim.lsp.enable("yami_lsp")
