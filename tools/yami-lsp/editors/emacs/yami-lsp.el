;;; yami-lsp.el --- Eglot configuration for YamiNet YLang/YSL -*- lexical-binding: t; -*-

;; Load this file from your init file. Eglot is built into Emacs 29 and later.
;; Replace "yami-lsp" with the absolute binary path if it is not on PATH.

(require 'eglot)

(define-derived-mode ylang-mode prog-mode "YLang"
  "Major mode for YamiNet YLang documents."
  (setq-local indent-tabs-mode nil)
  (setq-local comment-start nil))

(define-derived-mode ysl-mode prog-mode "YSL"
  "Major mode for YamiNet YSL stylesheets."
  (setq-local indent-tabs-mode nil)
  (setq-local comment-start nil))

(add-to-list 'auto-mode-alist '("\\.ylang\\'" . ylang-mode))
(add-to-list 'auto-mode-alist '("\\.ysl\\'" . ysl-mode))

(add-to-list 'eglot-server-programs '(ylang-mode . ("yami-lsp")))
(add-to-list 'eglot-server-programs '(ysl-mode . ("yami-lsp")))

;; Optional website directory. Eglot sends this with
;; workspace/didChangeConfiguration after initialize; when omitted the server
;; uses the project root and then the document's directory.
;; (setq-default eglot-workspace-configuration
;;               '(:yami (:siteRoot "/absolute/path/to/website")))

(provide 'yami-lsp)
;;; yami-lsp.el ends here
