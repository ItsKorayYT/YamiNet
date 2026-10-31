" Vim syntax file for YamiNet YSL 0.1/0.2.
" Install this file as ~/.vim/syntax/ysl.vim (or in a runtimepath package).
if exists("b:current_syntax")
  finish
endif

syntax match yslHeader /^@style\s\+0\.[12]\s*$/
syntax match yslVersion /0\.[12]/ contained
syntax match yslGroup /^\%(canvas\|text\|heading\|section\|link\|card\|image\|row\)\s*$/
syntax match yslProperty /^  \zs[a-z][a-z0-9-]*\ze\s/
syntax match yslColor /#[0-9A-Fa-f]\{6}\>/
syntax match yslNumber /\<\d\{1,4}\>/
syntax keyword yslKeyword left center right
syntax keyword yslWeight 400 600

highlight def link yslHeader Keyword
highlight def link yslVersion Number
highlight def link yslGroup Type
highlight def link yslProperty Identifier
highlight def link yslColor Constant
highlight def link yslNumber Number
highlight def link yslKeyword Keyword
highlight def link yslWeight Constant

let b:current_syntax = "ysl"
