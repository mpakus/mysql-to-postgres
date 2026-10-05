#!/usr/bin/env python3
"""Regenerate the attributed T21 patch from the pinned, read-only pgloader source."""

from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "pgloader" / "clojure"
OUTPUT = ROOT / "my2pg" / "tests" / "performance" / "patches" / "pgloader-t21-phase-intervals.patch"


def replace_once(text, before, after, label):
    if text.count(before) != 1:
        raise RuntimeError(f"expected one source match for {label}")
    return text.replace(before, after, 1)


with tempfile.TemporaryDirectory(prefix="pgloader-t21-patch-") as directory:
    work = Path(directory)
    for relative in ("src/pgloader/core.clj", "src/pgloader/summary.clj", "src/pgloader/load_file/ast.clj",
                     "test/pgloader/summary_test.clj", "test/pgloader/load_file/parser_test.clj"):
        target = work / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((SOURCE / relative).read_bytes())
    subprocess.run(["git", "init", "-q", str(work)], check=True)
    subprocess.run(["git", "-C", str(work), "add", "."], check=True)
    subprocess.run(["git", "-C", str(work), "-c", "user.name=My2pg", "-c", "user.email=my2pg@example.invalid",
                    "commit", "-qm", "pinned reference baseline"], check=True)

    summary_path = work / "src/pgloader/summary.clj"
    summary = summary_path.read_text()
    summary = replace_once(summary,
        '''(defn write-summary-json
  ([^String path verbose] (write-summary-json path verbose nil))
  ([^String path verbose wall-nanos]
   (let [data (reduce''',
        '''(defn write-summary-json
  ([^String path verbose] (write-summary-json path verbose nil))
  ([^String path verbose wall-nanos] (write-summary-json path verbose wall-nanos nil))
  ([^String path verbose wall-nanos benchmark-phases]
   (let [data (reduce''', "summary-json-arities")
    summary = replace_once(summary,
        '''     (spit path (json/write-str full :key-fn name)))))

(defn write-summary
  ([path verbose] (write-summary path verbose nil))
  ([path verbose wall-nanos]
   (when path
     (cond
       (str/ends-with? path ".csv")  (write-summary-csv path verbose wall-nanos)
       (str/ends-with? path ".json") (write-summary-json path verbose wall-nanos)
       :else                         (write-summary-csv path verbose wall-nanos))
     (println (str "Summary written to " path)))))''',
        '''     (spit path (json/write-str (cond-> full
                                 benchmark-phases (assoc :benchmark_phases benchmark-phases))
                                :key-fn name)))))

(defn write-summary
  ([path verbose] (write-summary path verbose nil))
  ([path verbose wall-nanos] (write-summary path verbose wall-nanos nil))
  ([path verbose wall-nanos benchmark-phases]
   (when path
     (cond
       (str/ends-with? path ".csv")  (write-summary-csv path verbose wall-nanos)
       (str/ends-with? path ".json") (write-summary-json path verbose wall-nanos benchmark-phases)
       :else                         (write-summary-csv path verbose wall-nanos))
     (println (str "Summary written to " path)))))''', "summary-json-output")
    summary_path.write_text(summary)

    core_path = work / "src/pgloader/core.clj"
    core = core_path.read_text()
    core = replace_once(core,
        '''          run-wall-t0 (System/nanoTime)
          source      (source-from-uri source-uri table-spec (:with-options cmd) source-overrides (:decoding-as cmd))''',
        '''          run-wall-t0 (System/nanoTime)
          catalog-phase-t0 (atom nil)
          phase-intervals (atom [])
          source      (source-from-uri source-uri table-spec (:with-options cmd) source-overrides (:decoding-as cmd))''',
        "run-phase-state")
    core = replace_once(core,
        '''                    fetch-t0   (System/nanoTime)
                    cat        (catalog source)''',
        '''                    fetch-t0   (System/nanoTime)
                    _          (reset! catalog-phase-t0 fetch-t0)
                    cat        (catalog source)''', "catalog-start")
    core = replace_once(core,
        '''                          (stats/update-entry! :pre "Create tables"
                                               :rows (count cat)
                                               :total-nanos (- (System/nanoTime) ddl-phase-start))))
          ;; Create the index stats entry before Phase 2 so futures can update it.''',
        '''                          (stats/update-entry! :pre "Create tables"
                                               :rows (count cat)
                                               :total-nanos (- (System/nanoTime) ddl-phase-start))))
                      (when-let [phase-t0 @catalog-phase-t0]
                        (swap! phase-intervals conj
                               {:kind "phase_interval" :phase "catalog"
                                :clock "run_monotonic_millis" :scope "run"
                                :start_elapsed_millis (quot (- phase-t0 run-wall-t0) 1000000)
                                :end_elapsed_millis (quot (- (System/nanoTime) run-wall-t0) 1000000)
                                :operation_classes ["source_mysql_catalog" "target_postgresql_schema_table_prepare"]}))
          ;; Create the index stats entry before Phase 2 so futures can update it.''', "catalog-end")
    core = replace_once(core,
        '''                          (stats/update-entry! :post "COPY Wall-Clock Time"
                                               :rows workers
                                               :bytes (:bytes (stats/get-totals :data))
                                               :total-nanos (- (System/nanoTime) copy-wall-t0))))''',
        '''                          (stats/update-entry! :post "COPY Wall-Clock Time"
                                               :rows workers
                                               :bytes (:bytes (stats/get-totals :data))
                                               :total-nanos (- (System/nanoTime) copy-wall-t0))
                          (swap! phase-intervals conj
                                 {:kind "phase_interval" :phase "copy"
                                  :clock "run_monotonic_millis" :scope "run"
                                  :start_elapsed_millis (quot (- copy-wall-t0 run-wall-t0) 1000000)
                                  :end_elapsed_millis (quot (- (System/nanoTime) run-wall-t0) 1000000)
                                  :operation_classes ["row_copy"]})))''', "copy-interval")
    core = replace_once(core,
        '''                                               :total-nanos (- (System/nanoTime) start)))))
                      (when (get with-options :reset-sequences false)''',
        '''                                               :total-nanos (- (System/nanoTime) start)))))
                      (when-let [phase-t0 @idx-wall-t0]
                        (swap! phase-intervals conj
                               {:kind "phase_interval" :phase "index_constraints"
                                :clock "run_monotonic_millis" :scope "run"
                                :start_elapsed_millis (quot (- phase-t0 run-wall-t0) 1000000)
                                :end_elapsed_millis (quot (- (System/nanoTime) run-wall-t0) 1000000)
                                :operation_classes ["primary_keys" "indexes" "foreign_keys" "check_constraints"]}))
                      (when (get with-options :reset-sequences false)''', "index-constraints-end")
    core = replace_once(core,
        '''            (summary/write-summary summary-path verbose wall-nanos)))))))''',
        '''            (summary/write-summary summary-path verbose wall-nanos @phase-intervals)))))))''',
        "summary-call")
    core_path.write_text(core)

    ast_path = work / "src/pgloader/load_file/ast.clj"
    ast = replace_once(ast_path.read_text(),
        ''':batch-size          [tag (second (second inner))]''',
        ''':batch-size          [tag (let [ds (second inner)
                                            n  (Long/parseLong (second (second ds)))
                                            u  (if (> (count ds) 2) (second (nth ds 2)) "B")]
                                        (str (* n (case u "KB" 1024 "MB" (* 1024 1024)
                                                        "GB" (* 1024 1024 1024) 1))))]''',
        "batch-size-byte-count")
    ast_path.write_text(ast)

    test_path = work / "test/pgloader/summary_test.clj"
    test_text = replace_once(test_path.read_text(),
        '''            [pgloader.stats :as stats]
            [pgloader.summary :as summary])''',
        '''            [pgloader.stats :as stats]
            [clojure.data.json :as json]
            [pgloader.summary :as summary])''', "summary-test-json-import")
    test_path.write_text(test_text + '''

(deftest test-summary-preserves-overlapping-benchmark-phase-intervals
  (testing "benchmark intervals are serialized without changing native totals"
    (stats/clear!)
    (stats/new-entry! :data "users")
    (stats/update-entry! :data "users" :rows 2 :bytes 8 :total-nanos 100)
    (let [file (java.io.File/createTempFile "pgloader-phase-" ".json")
          intervals [{:kind "phase_interval" :phase "copy"
                      :clock "run_monotonic_millis" :scope "run"
                      :start_elapsed_millis 10 :end_elapsed_millis 40
                      :operation_classes ["row_copy"]}
                     {:kind "phase_interval" :phase "index_constraints"
                      :clock "run_monotonic_millis" :scope "run"
                      :start_elapsed_millis 25 :end_elapsed_millis 50
                      :operation_classes ["primary_keys" "indexes" "foreign_keys" "check_constraints"]}]]
      (try
        (summary/write-summary (.getAbsolutePath file) false 100 intervals)
        (let [output (json/read-str (slurp file))]
          (is (= intervals (get output "benchmark_phases")))
          (is (= 2 (get-in output ["phases" "data" "total" "rows"])))
          (is (< (get-in (first (get output "benchmark_phases")) ["start_elapsed_millis"])
                 (get-in (second (get output "benchmark_phases")) ["end_elapsed_millis"]))))
        (finally (.delete file)))))
''')

    parser_test_path = work / "test/pgloader/load_file/parser_test.clj"
    parser_test_path.write_text(parser_test_path.read_text() + '''

(deftest test-parse-database-batch-size-as-bytes
  (testing "database batch sizes are converted to bytes for the runtime parser"
    (let [result (parser/parse-string
                  "LOAD DATABASE FROM mysql://user@localhost/mydb
                    INTO postgresql:///target
                    WITH batch rows = 25000, batch size = 16MB, prefetch rows = 2;")]
      (is (:ok result))
      (is (= 25000 (get-in result [:ok :with-options :batch-rows])))
      (is (= "16777216" (get-in result [:ok :with-options :batch-size])))
      (is (= 2 (get-in result [:ok :with-options :prefetch-rows])))))
  (testing "a bare batch size remains a byte count"
    (let [result (parser/parse-string
                  "LOAD DATABASE FROM mysql://user@localhost/mydb
                    INTO postgresql:///target
                    WITH batch size = 4096;")]
      (is (:ok result))
      (is (= "4096" (get-in result [:ok :with-options :batch-size]))))))
''')
    subprocess.run(["git", "-C", str(work), "add", "."], check=True)
    diff = subprocess.run(["git", "-C", str(work), "diff", "--cached", "--binary", "HEAD"],
                          check=True, capture_output=True, text=True).stdout
    header = ("From: My2pg contributors\n"
              "Purpose: expose T21 phase intervals and normalize pgloader-v4 batch-size AST values\n"
              "Reference: layerware/pgloader clojure runtime at 231ab86778ca5ffd7de40878714760c8b4860cdf\n"
              "License: this patch is independently authored; it copies no upstream code.\n\n")
    OUTPUT.write_text(header + diff)
