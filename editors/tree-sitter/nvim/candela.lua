-- candela support for Neovim: `.cdl` filetype detection, the tree-sitter
-- grammar, and the `candela-lsp` language server.
--
-- Copy this file to ~/.config/nvim/lua/candela.lua and call:
--
--     require('candela').setup()
--
-- See README.md in this directory for the options and for the query files.

local M = {}

local defaults = {
  -- Where the grammar comes from. Point `grammar_path` at a local clone of
  -- the candela repository to build the grammar from your checkout;
  -- otherwise the repository is fetched at `grammar_revision`, which tracks
  -- the default branch. Set it to a commit to pin the parser instead.
  grammar_url = 'https://github.com/lumen-fx/candela',
  grammar_revision = 'main',
  grammar_path = nil,

  -- Absolute path to `candela-lsp`. Unset means the bare name on $PATH,
  -- where the toolchain installs it.
  server_path = nil,

  treesitter = true,
  lsp = true,
}

local GRAMMAR_LOCATION = 'editors/tree-sitter'
local GRAMMAR_QUERIES = 'editors/tree-sitter/queries'

--- Resolve the `candela-lsp` command, mirroring the VS Code extension: an
--- explicit path wins, then the bare name on $PATH.
---@param opts table
---@return string
function M.server_command(opts)
  opts = vim.tbl_extend('force', defaults, opts or {})
  if opts.server_path and opts.server_path ~= '' then
    return vim.fn.expand(opts.server_path)
  end
  return vim.fn.has('win32') == 1 and 'candela-lsp.exe' or 'candela-lsp'
end

local function register_filetype()
  vim.filetype.add({ extension = { cdl = 'candela' } })
end

local function install_info(opts)
  local info = { location = GRAMMAR_LOCATION, queries = GRAMMAR_QUERIES }
  if opts.grammar_path then
    info.path = vim.fn.expand(opts.grammar_path)
  else
    info.url = opts.grammar_url
    info.revision = opts.grammar_revision
  end
  return info
end

local function register_parser(opts)
  local ok, parsers = pcall(require, 'nvim-treesitter.parsers')
  if not ok then
    return
  end

  if type(parsers.get_parser_configs) == 'function' then
    -- nvim-treesitter master branch.
    local info = install_info(opts)
    parsers.get_parser_configs().candela = {
      install_info = {
        url = info.path or info.url,
        revision = info.revision,
        location = GRAMMAR_LOCATION,
        files = { 'src/parser.c' },
      },
      filetype = 'candela',
    }
  else
    -- nvim-treesitter main branch. Registration has to run while the plugin
    -- refreshes its parser list, which is what the TSUpdate event is for.
    parsers.candela = { install_info = install_info(opts), tier = 2 }
  end
end

--- Set up filetype detection, the grammar, and the language server.
---@param opts? table
function M.setup(opts)
  opts = vim.tbl_extend('force', defaults, opts or {})

  register_filetype()

  if opts.treesitter then
    register_parser(opts)
    vim.api.nvim_create_autocmd('User', {
      pattern = 'TSUpdate',
      callback = function()
        register_parser(opts)
      end,
    })
  end

  if opts.lsp then
    vim.lsp.config('candela_lsp', {
      cmd = { M.server_command(opts) },
      filetypes = { 'candela' },
      root_markers = { '.git' },
    })
    vim.lsp.enable('candela_lsp')
  end
end

return M
