;;; history.scm — VFS file revision history plugin
;;;
;;; Automatically snapshots VFS files before writes. Provides tools for
;;; browsing history, viewing diffs, and reverting to previous revisions.
;;;
;;; Requires: unsandboxed tier ((harness io) + (chibi diff))
;;; Hook: pre_vfs_write — snapshots current content before overwrite
;;;
;;; Storage layout: <file-dir>/.chibi/history/<filename>/<N>  (revision N)
;;;                 <file-dir>/.chibi/history/<filename>/meta  (alist: ((next . N)))

(import (scheme base)
        (scheme write)
        (scheme read)
        (scheme char)
        (chibi diff)
        (harness io)
        (harness tools)
        (harness hooks))

;; --- constants ---

(define %history-keep% 10)
(define %history-prefix% ".chibi/history")

;; --- path helpers ---

;;; Split a VFS URI into (parent-dir . filename).
;;; "vfs:///shared/tool.scm" → ("vfs:///shared" . "tool.scm")
(define (split-vfs-path uri)
  (let* ((path (substring uri 6 (string-length uri)))  ;; strip "vfs://"
         (last-slash (let loop ((i (- (string-length path) 1)))
                       (cond ((< i 0) #f)
                             ((char=? (string-ref path i) #\/) i)
                             (else (loop (- i 1)))))))
    (if last-slash
        (cons (string-append "vfs://" (substring path 0 last-slash))
              (substring path (+ last-slash 1) (string-length path)))
        (cons "vfs://" path))))

;;; Build history directory path for a file.
;;; "vfs:///shared/tool.scm" → "vfs:///shared/.chibi/history/tool.scm"
(define (history-dir-for uri)
  (let ((parts (split-vfs-path uri)))
    (string-append (car parts) "/" %history-prefix% "/" (cdr parts))))

;;; Build path to a specific revision file.
(define (revision-path history-dir n)
  (string-append history-dir "/" (number->string n)))

;;; Build path to meta file.
(define (meta-path history-dir)
  (string-append history-dir "/meta"))

;; --- meta helpers ---

;;; Read meta alist from history dir. Returns ((next . N)) or default.
(define (read-meta history-dir)
  (let ((content (io-read (meta-path history-dir))))
    (if content
        (read (open-input-string content))
        '((next . 1)))))

;;; Write meta alist to history dir.
(define (write-meta! history-dir meta)
  (let ((out (open-output-string)))
    (write meta out)
    (io-write (meta-path history-dir) (get-output-string out))))

;;; Get a field from meta alist.
(define (meta-ref meta key)
  (cond ((assq key meta) => cdr)
        (else #f)))

;;; Set a field in meta alist (functional update).
(define (meta-set meta key value)
  (cons (cons key value)
        (filter (lambda (pair) (not (eq? (car pair) key))) meta)))

;; --- revision enumeration ---

;;; List revision numbers in a history dir, sorted ascending.
(define (list-revisions history-dir)
  (let ((entries (io-list history-dir)))
    (if (null? entries)
        '()
        (let loop ((es entries) (acc '()))
          (if (null? es)
              (isort acc <)
              (let ((name (car es)))
                (if (string=? name "meta")
                    (loop (cdr es) acc)
                    (let ((n (string->number name)))
                      (if n
                          (loop (cdr es) (cons n acc))
                          (loop (cdr es) acc))))))))))

;; isort helper (insertion sort, fine for <= keep+1 elements)
;; named isort to avoid shadowing any future (scheme base) sort
(define (isort lst less?)
  (define (insert x sorted)
    (cond ((null? sorted) (list x))
          ((less? x (car sorted)) (cons x sorted))
          (else (cons (car sorted) (insert x (cdr sorted))))))
  (let loop ((l lst) (acc '()))
    (if (null? l) acc
        (loop (cdr l) (insert (car l) acc)))))

;; --- pruning ---

;;; Delete oldest revisions until count <= keep.
(define (prune-revisions! history-dir keep)
  (let ((revs (list-revisions history-dir)))
    (when (> (length revs) keep)
      (let loop ((to-delete (- (length revs) keep)) (revs revs))
        (when (and (> to-delete 0) (pair? revs))
          (io-delete (revision-path history-dir (car revs)))
          (loop (- to-delete 1) (cdr revs)))))))

;; --- string helpers (defined before first use) ---

;; string-contains helper (not in R7RS base)
(define (string-contains haystack needle)
  (let ((hlen (string-length haystack))
        (nlen (string-length needle)))
    (let loop ((i 0))
      (cond ((> (+ i nlen) hlen) #f)
            ((string=? (substring haystack i (+ i nlen)) needle) #t)
            (else (loop (+ i 1)))))))

;; string-join helper
(define (string-join lst sep)
  (if (null? lst) ""
      (let loop ((rest (cdr lst)) (acc (car lst)))
        (if (null? rest) acc
            (loop (cdr rest) (string-append acc sep (car rest)))))))

;; --- hook callback ---

;;; pre_vfs_write hook: snapshot current file content before overwrite.
;;; Best-effort: errors are caught and silently ignored (advisory, not a gate).
(define (on-pre-vfs-write payload)
  (guard (exn (#t #f))  ;; catch all errors, return #f (no-op)
    (let ((path (cdr (assoc "path" payload))))
      ;; Skip non-VFS paths and our own metadata writes
      (when (and (string? path)
                 (>= (string-length path) 6)
                 (string=? (substring path 0 6) "vfs://")
                 (not (string-contains path "/.chibi/")))
        (let ((current (io-read path)))
          ;; Only snapshot if file already exists (not a new file)
          (when current
            (let* ((hdir (history-dir-for path))
                   (meta (read-meta hdir))
                   (next (or (meta-ref meta 'next) 1)))
              ;; Write snapshot
              (io-write (revision-path hdir next) current)
              ;; Update meta
              (write-meta! hdir (meta-set meta 'next (+ next 1)))
              ;; Prune old revisions
              (prune-revisions! hdir %history-keep%))))))))

;; Register the hook
(register-hook 'pre_vfs_write on-pre-vfs-write)

;; --- tools ---

(define-tool file_history_log
  (description "List revision history for a VFS file")
  (parameters (list
    (cons "path" '(("type" . "string")
                   ("description" . "VFS path (e.g. vfs:///shared/tool.scm)")))))
  (execute (lambda (args)
    (let* ((path (cdr (assoc "path" args)))
           (hdir (history-dir-for path))
           (revs (list-revisions hdir)))
      (if (null? revs)
          (string-append "No history for " path)
          (let ((lines (map (lambda (n)
                              (string-append "  @" (number->string n)))
                            (reverse revs))))  ;; newest first
            (string-append "Revisions for " path ":\n"
                           (string-join lines "\n"))))))))

(define-tool file_history_show
  (description "Show file content at a specific revision")
  (parameters (list
    (cons "path" '(("type" . "string")
                   ("description" . "VFS path")))
    (cons "revision" '(("type" . "integer")
                       ("description" . "Revision number")))))
  (execute (lambda (args)
    (let* ((path (cdr (assoc "path" args)))
           (rev (cdr (assoc "revision" args)))
           (rev-n (if (string? rev) (string->number rev) rev))
           (hdir (history-dir-for path))
           (content (io-read (revision-path hdir rev-n))))
      (if content
          content
          (string-append "Revision @" (number->string rev-n)
                         " not found for " path))))))

(define-tool file_history_diff
  (description "Show diff between current file and a revision")
  (parameters (list
    (cons "path" '(("type" . "string")
                   ("description" . "VFS path")))
    (cons "revision" '(("type" . "integer")
                       ("description" . "Revision to diff against (default: most recent)")
                       ("required" . #f)))))
  (execute (lambda (args)
    (let* ((path (cdr (assoc "path" args)))
           (hdir (history-dir-for path))
           (rev-arg (assoc "revision" args))
           (revs (list-revisions hdir)))
      (if (null? revs)
          (string-append "No history for " path)
          (let* ((rev-n (if (and rev-arg (cdr rev-arg))
                            (let ((v (cdr rev-arg)))
                              (if (string? v) (string->number v) v))
                            (car (reverse revs))))  ;; default: most recent
                 (snapshot (io-read (revision-path hdir rev-n)))
                 (current (io-read path)))
            (cond
              ((not snapshot)
               (string-append "Revision @" (number->string rev-n)
                              " not found for " path))
              ((not current)
               (string-append "File " path " no longer exists"))
              ((string=? snapshot current)
               (string-append "No changes since revision @"
                              (number->string rev-n)))
              (else
               (let ((d (diff snapshot current)))
                 (string-append "Diff " path " @" (number->string rev-n)
                                " vs current:\n"
                                (diff->string d)))))))))))

(define-tool file_history_revert
  (description "Revert a VFS file to a previous revision")
  (parameters (list
    (cons "path" '(("type" . "string")
                   ("description" . "VFS path")))
    (cons "revision" '(("type" . "integer")
                       ("description" . "Revision number to restore")))))
  (execute (lambda (args)
    (let* ((path (cdr (assoc "path" args)))
           (rev (cdr (assoc "revision" args)))
           (rev-n (if (string? rev) (string->number rev) rev))
           (hdir (history-dir-for path))
           (snapshot (io-read (revision-path hdir rev-n))))
      (if (not snapshot)
          (string-append "Revision @" (number->string rev-n)
                         " not found for " path)
          (begin
            ;; Write via call-tool so the hook fires and snapshots pre-revert state
            (call-tool "write_file"
                       (list (cons "path" path)
                             (cons "content" snapshot)))
            (string-append "Reverted " path " to revision @"
                           (number->string rev-n))))))))
