" Vim syntax file for YamiNet YLang 0.1/0.2.
" Install this file as ~/.vim/syntax/ylang.vim (or in a runtimepath package).
if exists("b:current_syntax")
  finish
endif

syntax match ylangHeader /^@page\s\+0\.[12]\s*$/
syntax match ylangVersion /0\.[12]/ contained
syntax match ylangMetadata /^  \(title\|language\)\ze\s/
syntax match ylangMetadataValue /^  \(title\|language\)\s\+\zs.*$/
syntax match ylangHeading /^=\{1,6}\ze\s/
syntax match ylangSectionMarker /^::/ nextgroup=ylangSectionName skipwhite
syntax match ylangSectionName /\S\+/ contained
syntax match ylangContainer /^::\s\+\%(card\|row\|group\|list\|steps\)\>/
syntax match ylangLink /^>\s\+\S\+/
syntax match ylangLinkLabel /|\s\+\zs.*$/
syntax match ylangImage /^!\s\+\S\+/
syntax match ylangSeparator /^---\s*$/
syntax match ylangEscape /^\\/
syntax region ylangStrong start=/\[strong:\s/ end=/\]/ keepend
syntax region ylangEmphasis start=/\[em:\s/ end=/\]/ keepend
syntax region ylangCode start=/\[code:\s/ end=/\]/ keepend
syntax match ylangInlineMarker /\[\(strong\|em\|code\):/ contained

highlight def link ylangHeader Keyword
highlight def link ylangVersion Number
highlight def link ylangMetadata Identifier
highlight def link ylangMetadataValue String
highlight def link ylangHeading Title
highlight def link ylangSectionMarker Delimiter
highlight def link ylangSectionName Function
highlight def link ylangContainer Keyword
highlight def link ylangLink Underlined
highlight def link ylangLinkLabel String
highlight def link ylangImage Special
highlight def link ylangSeparator Delimiter
highlight def link ylangEscape String
highlight def link ylangStrong Bold
highlight def link ylangEmphasis Italic
highlight def link ylangCode String
highlight def link ylangInlineMarker Type

let b:current_syntax = "ylang"
