" Filetype detection for YamiNet source files.
augroup yami_filetype
  autocmd!
  autocmd BufRead,BufNewFile *.ylang setfiletype ylang
  autocmd BufRead,BufNewFile *.ysl setfiletype ysl
augroup END
