;;; tasks.scm — structured task management plugin for chibi~
;;;
;;; Exposes five tools: task_create, task_update, task_view, task_list, task_delete.
;;; Tasks are stored as .task files in the VFS at:
;;;   /home/<ctx>/tasks/         (context-local)
;;;   /flocks/<name>/tasks/      (flock-shared, prefix path with "flock:<name>/")
;;;
;;; Each .task file contains two scheme datums:
;;;   1. metadata alist  — id, status, priority, depends-on, assigned-to, created, updated
;;;   2. body string     — freeform task description (optional)
;;;
;;; Install to VFS at /tools/shared/tasks.scm to make tools available globally.

(import (scheme base))
(import (scheme char))
(import (harness tools))

;;; ---- helpers ---------------------------------------------------------------

;;; Split string on a character delimiter, returning a list of substrings.
(define (string-split str ch)
  (let loop ((i 0) (start 0) (acc '()))
    (cond
      ((= i (string-length str))
       (reverse (cons (substring str start i) acc)))
      ((char=? (string-ref str i) ch)
       (loop (+ i 1) (+ i 1) (cons (substring str start i) acc)))
      (else
       (loop (+ i 1) start acc)))))

;;; Trim leading and trailing whitespace from a string.
(define (string-trim-both s)
  (let* ((len (string-length s))
         (start (let scan ((i 0))
                  (if (or (= i len)
                          (not (char-whitespace? (string-ref s i))))
                      i
                      (scan (+ i 1)))))
         (end (let scan ((i (- len 1)))
                (if (or (< i start)
                        (not (char-whitespace? (string-ref s i))))
                    (+ i 1)
                    (scan (- i 1))))))
    (substring s start end)))

;;; Escape a string for writing as a scheme double-quoted datum.
(define (escape-string s)
  (let loop ((i 0) (out '()))
    (if (= i (string-length s))
        (list->string (reverse out))
        (let ((c (string-ref s i)))
          (cond
            ((char=? c #\") (loop (+ i 1) (cons #\" (cons #\\ out))))
            ((char=? c #\\) (loop (+ i 1) (cons #\\ (cons #\\ out))))
            (else (loop (+ i 1) (cons c out))))))))

;;; Serialise a metadata alist + body to .task file content (two datums).
;;; meta is a list of (key . value) pairs; symbol-valued fields (status, priority)
;;; are written as symbols, string-valued fields as quoted strings.
(define (serialise-task meta body)
  (string-append
    "(" (meta->sexp meta) ")\n\n"
    "\"" (escape-string body) "\"\n"))

;;; Render metadata alist fields as s-expression inner pairs.
(define (meta->sexp meta)
  (let loop ((pairs meta) (out ""))
    (if (null? pairs)
        out
        (let* ((pair  (car pairs))
               (key   (car pair))
               (val   (cdr pair))
               (sep   (if (string=? out "") "" "\n "))
               (entry
                 (cond
                   ;; depends-on is a flat list: (depends-on "id1" "id2")
                   ((string=? key "depends-on")
                    (string-append "(" key
                      (let dep-loop ((deps val) (s ""))
                        (if (null? deps) s
                            (dep-loop (cdr deps)
                                      (string-append s " \"" (car deps) "\""))))
                      ")"))
                   ;; symbol values: status, priority
                   ((or (string=? key "status") (string=? key "priority"))
                    (string-append "(" key " . " val ")"))
                   ;; string values: id, assigned-to, created, updated
                   (else
                    (string-append "(" key " . \"" (escape-string val) "\")")))))
          (loop (cdr pairs) (string-append out sep entry))))))

;;; Resolve the VFS base directory and sub-path for a given task path arg.
;;; If path starts with "flock:<name>/" routes to /flocks/<name>/tasks/.
;;; Otherwise routes to /home/<context-name>/tasks/.
;;; Returns two values: (base-dir-string sub-path-string).
(define (resolve-task-base path)
  (if (and (>= (string-length path) 6)
           (string=? (substring path 0 6) "flock:"))
      (let* ((rest  (substring path 6 (string-length path)))
             (slash (let scan ((i 0))
                      (if (or (= i (string-length rest))
                              (char=? (string-ref rest i) #\/))
                          i
                          (scan (+ i 1)))))
             (flock (substring rest 0 slash))
             (sub   (if (= slash (string-length rest)) ""
                        (substring rest (+ slash 1) (string-length rest)))))
        (values (string-append "/flocks/" flock "/tasks") sub))
      (values (string-append "/home/" %context-name% "/tasks") path)))

;;; Check if a string contains a substring (linear scan).
(define (string-contains? haystack needle)
  (let* ((hlen (string-length haystack))
         (nlen (string-length needle))
         (limit (- hlen nlen)))
    (if (< limit 0)
        #f
        (let loop ((i 0))
          (cond
            ((> i limit) #f)
            ((string=? (substring haystack i (+ i nlen)) needle) #t)
            (else (loop (+ i 1))))))))

;;; Extract the value of a symbol-valued alist field like "(field . value)" from content.
;;; Returns the value string or #f if not found.
(define (extract-symbol-field content field)
  (let* ((prefix (string-append "(" field " . "))
         (plen   (string-length prefix))
         (clen   (string-length content)))
    (let loop ((i 0))
      (cond
        ((> (+ i plen) clen) #f)
        ((string=? (substring content i (+ i plen)) prefix)
         ;; found prefix — read until closing paren
         (let val-loop ((j (+ i plen)) (out '()))
           (cond
             ((>= j clen) (list->string (reverse out)))
             ((char=? (string-ref content j) #\))
              (list->string (reverse out)))
             (else (val-loop (+ j 1) (cons (string-ref content j) out))))))
        (else (loop (+ i 1)))))))

;;; Extract the value of a string-valued alist field like '(field . "value")' from content.
;;; Returns the quoted value (including surrounding quotes) or #f if not found.
;;; The full match string for replacement is (string-append "(" field " . " result ")").
(define (extract-string-field content field)
  (let* ((prefix (string-append "(" field " . \""))
         (plen   (string-length prefix))
         (clen   (string-length content)))
    (let loop ((i 0))
      (cond
        ((> (+ i plen) clen) #f)
        ((string=? (substring content i (+ i plen)) prefix)
         ;; found prefix — read until closing quote (handle escaped quotes)
         (let val-loop ((j (+ i plen)) (out (list #\")))
           (cond
             ((>= j clen) #f)  ; unterminated string
             ((and (char=? (string-ref content j) #\\)
                   (< (+ j 1) clen))
              ;; escaped char — consume both
              (val-loop (+ j 2) (cons (string-ref content (+ j 1))
                                      (cons #\\ out))))
             ((char=? (string-ref content j) #\")
              (list->string (reverse (cons #\" out))))
             (else (val-loop (+ j 1) (cons (string-ref content j) out))))))
        (else (loop (+ i 1)))))))

;;; List .task files recursively under a VFS directory.
;;; vfs_list returns lines of the form "name (file)" or "name (dir)".
;;; Returns a list of full VFS path strings (without vfs:// prefix).
(define (list-tasks-recursive dir)
  (let ((listing
          (call-tool "vfs_list"
            `(("path" . ,(string-append "vfs://" dir))))))
    (if (or (string=? listing "")
            (string=? listing "No entries found."))
        '()
        (let loop ((lines (string-split listing #\newline)) (acc '()))
          (if (null? lines)
              (reverse acc)
              (let* ((line (string-trim-both (car lines))))
                (cond
                  ((string=? line "")
                   (loop (cdr lines) acc))
                  ;; directory entry: "name (dir)"
                  ((and (>= (string-length line) 5)
                        (string=? (substring line (- (string-length line) 5) (string-length line)) "(dir)"))
                   (let ((name (string-trim-both
                                 (substring line 0 (- (string-length line) 5)))))
                     (loop (cdr lines)
                           (append (reverse (list-tasks-recursive (string-append dir "/" name)))
                                   acc))))
                  ;; file entry: "name (file)" — include only .task files
                  ((and (>= (string-length line) 6)
                        (string=? (substring line (- (string-length line) 6) (string-length line)) "(file)"))
                   (let ((name (string-trim-both
                                 (substring line 0 (- (string-length line) 6)))))
                     (if (and (>= (string-length name) 5)
                              (string=? (substring name (- (string-length name) 5) (string-length name)) ".task"))
                         (loop (cdr lines) (cons (string-append dir "/" name) acc))
                         (loop (cdr lines) acc))))
                  (else
                   (loop (cdr lines) acc)))))))))

;;; Read a .task file via file_head. Returns the content string or #f on error.
(define (read-task-file path)
  (let ((result
          (call-tool "file_head"
            `(("path"  . ,(string-append "vfs://" path))
              ("lines" . "200")))))
    ;; file_head returns an error message string on failure
    (if (or (string=? result "")
            (string-contains? result "Error")
            (string-contains? result "No such"))
        #f
        result)))

;;; Collect all task directories visible to the current context:
;;; the context-local dir plus member flock task dirs (via flock_list builtin).
(define (all-task-dirs)
  (let* ((local-dir (string-append "/home/" %context-name% "/tasks"))
         (raw (call-tool "flock_list" '()))
         (flock-dirs
           (if (or (string=? raw "") (string-contains? raw "Error"))
               '()
               (map (lambda (name)
                      (string-append "/flocks/" (string-trim-both name) "/tasks"))
                    (filter (lambda (s) (not (string=? (string-trim-both s) "")))
                            (string-split raw #\newline))))))
    (cons local-dir flock-dirs)))

;;; Find a .task file by ID by scanning all visible task directories.
;;; Returns the full VFS path (without vfs:// prefix) or #f if not found.
(define (find-task-by-id task-id)
  (let loop-dirs ((dirs (all-task-dirs)))
    (if (null? dirs)
        #f
        (let ((files (list-tasks-recursive (car dirs))))
          (let loop-files ((files files))
            (if (null? files)
                (loop-dirs (cdr dirs))
                (let ((content (read-task-file (car files))))
                  (if content
                      (let ((file-id (extract-string-field content "id")))
                        ;; extract-string-field returns the quoted value including
                        ;; surrounding quotes, e.g. "\"ab12\"" — compare inner value.
                        (if (and file-id
                                 (string=? (substring file-id 1 (- (string-length file-id) 1))
                                           task-id))
                            (car files)
                            (loop-files (cdr files))))
                      (loop-files (cdr files))))))))))

;;; Ensure a VFS directory exists (ignores error if already present).
(define (vfs-mkdir-safe dir)
  (call-tool "vfs_mkdir" `(("path" . ,(string-append "vfs://" dir)))))

;;; Create any intermediate directories for a path like "a/b/c".
(define (vfs-mkdir-parents base sub)
  (let ((parts (string-split sub #\/)))
    (let mk ((parts parts) (cur base))
      (if (or (null? parts) (null? (cdr parts)))
          #t
          (let ((next (string-append cur "/" (car parts))))
            (vfs-mkdir-safe next)
            (mk (cdr parts) next))))))

;;; ---- tools -----------------------------------------------------------------

(define-tool task_create
  (description "Create a new task file. Returns the task ID and VFS path. Use 'flock:<name>/sub/path' for flock-scoped tasks.")
  (parameters '((path . ((type . "string")
                          (description . "Task path relative to tasks root, e.g. 'auth/login' or 'flock:infra/deploy'. Must include a filename — 'flock:name' alone is invalid. Directories are created automatically.")))
                (body . ((type . "string")
                         (description . "Task description (plain text, can be multi-line). Optional.")))
                (priority . ((type . "string")
                             (description . "low, medium, or high. Defaults to medium.")))
                (assigned-to . ((type . "string")
                                (description . "Context name to assign this task to. Optional.")))
                (depends-on . ((type . "string")
                               (description . "Comma-separated task IDs this task depends on. Optional.")))))
  (execute (lambda (args)
    (let* ((path-arg  (cdr (assoc "path" args)))
           (body      (let ((b (assoc "body" args)))     (if b (cdr b) "")))
           (priority  (let ((p (assoc "priority" args))) (if p (cdr p) "medium")))
           (assigned  (assoc "assigned-to" args))
           (deps-str  (assoc "depends-on" args))
           (id        (generate-id))
           (ts        (current-timestamp))
           (deps      (if deps-str
                          (filter (lambda (s) (not (string=? s "")))
                                  (map string-trim-both
                                       (string-split (cdr deps-str) #\,)))
                          '()))
           (meta      (append
                        (list (cons "id" id)
                              (cons "status" "pending")
                              (cons "priority" priority))
                        (if assigned (list (cons "assigned-to" (cdr assigned))) '())
                        (if (null? deps) '() (list (cons "depends-on" deps)))
                        (list (cons "created" ts)
                              (cons "updated" ts)))))
      (let-values (((base sub) (resolve-task-base path-arg)))
        (if (string=? sub "")
            "error: task path must include a filename, e.g. 'auth/login' or 'flock:infra/deploy'"
        (let ((full-path (string-append base "/" sub ".task")))
          ;; Ensure base dir and any intermediate subdirs exist
          (vfs-mkdir-safe base)
          (if (string-contains? sub "/")
              (vfs-mkdir-parents base sub)
              #t)
          ;; Write the task file
          (call-tool "write_file"
            `(("path"    . ,(string-append "vfs://" full-path))
              ("content" . ,(serialise-task meta body))))
          (string-append "created task " id " at " full-path))))))))

(define-tool task_update
  (description "Update an existing task by ID. Finds the task by scanning task directories and updates specified fields.")
  (parameters '((id . ((type . "string")
                        (description . "Task ID to update.")))
                (status . ((type . "string")
                           (description . "New status: pending, in-progress, or done.")))
                (priority . ((type . "string")
                             (description . "New priority: low, medium, or high.")))
                (body . ((type . "string")
                         (description . "New task body, replaces existing body.")))
                (assigned-to . ((type . "string")
                                (description . "New assigned context name.")))))
  (execute (lambda (args)
    (let* ((task-id (cdr (assoc "id" args)))
           (path    (find-task-by-id task-id)))
      (if (not path)
          (string-append "error: task " task-id " not found")
          (let* ((vfs-uri       (string-append "vfs://" path))
                 (ts            (current-timestamp))
                 (content       (read-task-file path))
                 (new-status    (assoc "status" args))
                 (new-priority  (assoc "priority" args))
                 (new-body      (assoc "body" args))
                 (new-assigned  (assoc "assigned-to" args)))
            (if (not content)
                (string-append "error: could not read " path)
                (begin
                  ;; Update status: extract current value, do single targeted replace.
                  (if new-status
                      (let ((old-status (extract-symbol-field content "status")))
                        (if old-status
                            (call-tool "file_edit"
                              `(("path"      . ,vfs-uri)
                                ("operation" . "replace_string")
                                ("find"      . ,(string-append "(status . " old-status ")"))
                                ("replace"   . ,(string-append "(status . " (cdr new-status) ")"))))
                            #f))
                      #f)
                  ;; Update priority: same pattern.
                  (if new-priority
                      (let ((old-priority (extract-symbol-field content "priority")))
                        (if old-priority
                            (call-tool "file_edit"
                              `(("path"      . ,vfs-uri)
                                ("operation" . "replace_string")
                                ("find"      . ,(string-append "(priority . " old-priority ")"))
                                ("replace"   . ,(string-append "(priority . " (cdr new-priority) ")"))))
                            #f))
                      #f)
                  ;; Re-read content after status/priority mutations so body
                  ;; and timestamp edits operate on current on-disk state.
                  (let ((content (if (or new-status new-priority)
                                     (or (read-task-file path) content)
                                     content)))
                    ;; Replace body: find the closing ) of the alist and replace everything after.
                    (if new-body
                        (let ((tail (let find-tail ((i 0) (depth 0))
                                      (if (= i (string-length content))
                                          #f
                                          (let ((ch (string-ref content i)))
                                            (cond
                                              ((char=? ch #\() (find-tail (+ i 1) (+ depth 1)))
                                              ((char=? ch #\))
                                               (if (= depth 1)
                                                   (substring content i (string-length content))
                                                   (find-tail (+ i 1) (- depth 1))))
                                              (else (find-tail (+ i 1) depth))))))))
                          (if tail
                              (call-tool "file_edit"
                                `(("path"      . ,vfs-uri)
                                  ("operation" . "replace_string")
                                  ("find"      . ,tail)
                                  ("replace"   . ,(string-append ")\n\n\"" (escape-string (cdr new-body)) "\"\n"))))
                              #f))
                        #f)
                    ;; Always bump the updated timestamp.
                    (let ((content (if new-body
                                       (or (read-task-file path) content)
                                       content)))
                      (let ((old-updated (extract-string-field content "updated")))
                        (if old-updated
                            (call-tool "file_edit"
                              `(("path"      . ,vfs-uri)
                                ("operation" . "replace_string")
                                ("find"      . ,(string-append "(updated . " old-updated ")"))
                                ("replace"   . ,(string-append "(updated . \"" ts "\")"))))
                            #f))))
                  (string-append "updated task " task-id " at " path)))))))))

(define-tool task_view
  (description "View a task by ID. Returns full metadata and body.")
  (parameters '((id . ((type . "string")
                        (description . "Task ID to view.")))))
  (execute (lambda (args)
    (let* ((task-id (cdr (assoc "id" args)))
           (path    (find-task-by-id task-id)))
      (if (not path)
          (string-append "error: task " task-id " not found")
          (let ((content (read-task-file path)))
            (if (not content)
                (string-append "error: could not read task at " path)
                (string-append "path: " path "\n" content))))))))

(define-tool task_list
  (description "List tasks with optional filters. Shows all tasks from context task directories.")
  (parameters '((status . ((type . "string")
                            (description . "Filter by status: pending, in-progress, or done.")))
                (priority . ((type . "string")
                             (description . "Filter by priority: low, medium, or high.")))
                (assigned-to . ((type . "string")
                                (description . "Filter by assigned context name.")))))
  (execute (lambda (args)
    (let* ((filter-status   (assoc "status" args))
           (filter-priority (assoc "priority" args))
           (filter-assigned (assoc "assigned-to" args))
           (all-files       (let loop ((dirs (all-task-dirs)) (acc '()))
                              (if (null? dirs)
                                  acc
                                  (loop (cdr dirs)
                                        (append acc (list-tasks-recursive (car dirs))))))))
      (if (null? all-files)
          "no tasks found"
          (let loop ((files all-files) (out "") (count 0))
            (if (null? files)
                (if (= count 0) "no tasks match filters" out)
                (let* ((path    (car files))
                       (content (read-task-file path))
                       (matches (and content
                                     (or (not filter-status)
                                         (string-contains? content (cdr filter-status)))
                                     (or (not filter-priority)
                                         (string-contains? content (cdr filter-priority)))
                                     (or (not filter-assigned)
                                         (string-contains? content (cdr filter-assigned))))))
                  (loop (cdr files)
                        (if matches
                            (string-append out
                              (if (string=? out "") "" "\n---\n")
                              "path: " path "\n" content)
                            out)
                        (if matches (+ count 1) count))))))))))

(define-tool task_delete
  (description "Delete a task by ID. Removes the .task file from the VFS.")
  (parameters '((id . ((type . "string")
                        (description . "Task ID to delete.")))))
  (execute (lambda (args)
    (let* ((task-id (cdr (assoc "id" args)))
           (path    (find-task-by-id task-id)))
      (if (not path)
          (string-append "error: task " task-id " not found")
          (begin
            (call-tool "vfs_delete"
              `(("path" . ,(string-append "vfs://" path))))
            (string-append "deleted task " task-id " at " path)))))))
