// Export everything a decompilation project needs from an analyszed Ghidra program:
//   functions.csv  - every function: address, size, name, namespace, signature, flags
//   classes.csv    - every RTTI/namespace class with its vftable address(es)
//   strings.csv    - every defined string with its xref count
//   calls.csv      - static call graph edges (caller -> callee)
//   pseudo/        - decompiled pseudo-C, one file per class / address window
//   <name>.gzf     - the whole program database, importable into Ghidra on any OS
//
// Run headless:  analyzeHeadless <proj> <name> -process JustCause2.exe -noanalysis
//                -scriptPath <this dir> -postScript ExportDecompProject.java <outDir>
//@category Decomp
//@author jc2-decomp

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import ghidra.app.decompiler.*;
import ghidra.app.decompiler.parallel.*;
import ghidra.app.script.GhidraScript;
import ghidra.app.util.exporter.GzfExporter;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.DataType;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import ghidra.program.util.DefinedDataIterator;
import ghidra.util.task.TaskMonitor;

public class ExportDecompProject extends GhidraScript {

    private File outDir;

    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        outDir = new File(args.length > 0 ? args[0] : "export");
        outDir.mkdirs();
        println("ExportDecompProject -> " + outDir.getAbsolutePath());

        exportFunctions();
        exportClasses();
        exportStrings();
        exportCalls();
        exportPseudoC();
        exportGzf();
        println("ExportDecompProject done.");
    }

    private static String csv(String s) {
        if (s == null) return "";
        if (s.contains(",") || s.contains("\"") || s.contains("\n")) {
            return "\"" + s.replace("\"", "\"\"") + "\"";
        }
        return s;
    }

    private static String nsPath(Namespace ns) {
        if (ns == null || ns.isGlobal()) return "";
        return ns.getName(true);
    }

    // ---------------------------------------------------------------- functions.csv
    private void exportFunctions() throws IOException {
        FunctionManager fm = currentProgram.getFunctionManager();
        int n = 0;
        try (PrintWriter w = new PrintWriter(new File(outDir, "functions.csv"), StandardCharsets.UTF_8)) {
            w.println("address,size,name,namespace,signature,thunk,external,cconv,params,stack_purge,source");
            for (Function f : fm.getFunctions(true)) {
                if (f.isExternal()) continue;
                String sig;
                try { sig = f.getSignature().getPrototypeString(); } catch (Exception e) { sig = ""; }
                w.println(String.join(",",
                    "0x" + f.getEntryPoint().toString(),
                    Long.toString(f.getBody().getNumAddresses()),
                    csv(f.getName()),
                    csv(nsPath(f.getParentNamespace())),
                    csv(sig),
                    f.isThunk() ? "1" : "0",
                    f.isExternal() ? "1" : "0",
                    csv(f.getCallingConventionName()),
                    Integer.toString(f.getParameterCount()),
                    Integer.toString(f.getStackPurgeSize()),
                    csv(f.getSymbol() != null ? f.getSymbol().getSource().toString() : "")));
                n++;
            }
        }
        println("functions.csv: " + n + " functions");
    }

    // ---------------------------------------------------------------- classes.csv
    private void exportClasses() throws IOException {
        SymbolTable st = currentProgram.getSymbolTable();
        int n = 0;
        try (PrintWriter w = new PrintWriter(new File(outDir, "classes.csv"), StandardCharsets.UTF_8)) {
            w.println("class,parent_namespace,vftables,member_functions");
            Iterator<GhidraClass> it = st.getClassNamespaces();
            while (it.hasNext()) {
                GhidraClass gc = it.next();
                List<String> vt = new ArrayList<>();
                for (Symbol s : st.getSymbols(gc)) {
                    String nm = s.getName();
                    if (nm.startsWith("vftable") && !nm.contains("meta")) {
                        vt.add("0x" + s.getAddress().toString());
                    }
                }
                int members = 0;
                for (Symbol s : st.getSymbols(gc)) {
                    if (s.getSymbolType() == SymbolType.FUNCTION) members++;
                }
                w.println(String.join(",",
                    csv(gc.getName()),
                    csv(nsPath(gc.getParentNamespace())),
                    csv(String.join(";", vt)),
                    Integer.toString(members)));
                n++;
            }
        }
        println("classes.csv: " + n + " classes");
    }

    // ---------------------------------------------------------------- strings.csv
    private void exportStrings() throws IOException {
        ReferenceManager rm = currentProgram.getReferenceManager();
        int n = 0;
        try (PrintWriter w = new PrintWriter(new File(outDir, "strings.csv"), StandardCharsets.UTF_8)) {
            w.println("address,xrefs,type,value");
            DefinedDataIterator di = DefinedDataIterator.byDataInstance(currentProgram,
                d -> d.getDataType() instanceof ghidra.program.model.data.AbstractStringDataType);
            while (di.hasNext()) {
                Data d = di.next();
                Object v = d.getValue();
                if (v == null) continue;
                String s = v.toString();
                if (s.length() < 3) continue;
                int xrefs = rm.getReferenceCountTo(d.getAddress());
                w.println(String.join(",",
                    "0x" + d.getAddress().toString(),
                    Integer.toString(xrefs),
                    csv(d.getDataType().getName()),
                    csv(s)));
                n++;
            }
        }
        println("strings.csv: " + n + " strings");
    }

    // ---------------------------------------------------------------- calls.csv
    private void exportCalls() throws IOException {
        FunctionManager fm = currentProgram.getFunctionManager();
        long n = 0;
        try (PrintWriter w = new PrintWriter(new File(outDir, "calls.csv"), StandardCharsets.UTF_8)) {
            w.println("caller,callee");
            for (Function f : fm.getFunctions(true)) {
                if (f.isExternal()) continue;
                for (Function c : f.getCalledFunctions(monitor)) {
                    w.println("0x" + f.getEntryPoint() + ",0x" + c.getEntryPoint());
                    n++;
                }
                if (monitor.isCancelled()) break;
            }
        }
        println("calls.csv: " + n + " edges");
    }

    // ---------------------------------------------------------------- pseudo/
    private String groupFile(Function f) {
        Namespace ns = f.getParentNamespace();
        if (ns != null && !ns.isGlobal()) {
            String p = nsPath(ns).replace("::", "/").replaceAll("[^A-Za-z0-9_/]", "_");
            return "pseudo/" + p + ".cpp";
        }
        long a = f.getEntryPoint().getOffset();
        if (f.getName().startsWith("FUN_") || f.getName().startsWith("thunk_FUN_")) {
            return String.format("pseudo/_unnamed/text_%08x.c", a & ~0xFFFFL);
        }
        return String.format("pseudo/_global/named_%08x.c", a & ~0xFFFFFL);
    }

    private static final class Result {
        final Function f; final String text; final boolean ok;
        Result(Function f, String text, boolean ok) { this.f = f; this.text = text; this.ok = ok; }
    }

    private void exportPseudoC() throws Exception {
        FunctionManager fm = currentProgram.getFunctionManager();
        List<Function> all = new ArrayList<>();
        for (Function f : fm.getFunctions(true)) {
            if (f.isExternal() || f.isThunk()) continue;
            all.add(f);
        }
        println("pseudo/: decompiling " + all.size() + " functions");
        final int CHUNK = 1500;
        int done = 0, failed = 0;
        long t0 = System.currentTimeMillis();

        DecompileConfigurer configurer = new DecompileConfigurer() {
            @Override
            public void configure(DecompInterface decompiler) {
                DecompileOptions opts = new DecompileOptions();
                opts.setCommentStyle(DecompileOptions.CommentStyleEnum.CPPStyle);
                decompiler.setOptions(opts);
                decompiler.toggleCCode(true);
                decompiler.toggleSyntaxTree(false);
                decompiler.setSimplificationStyle("decompile");
            }
        };

        for (int i = 0; i < all.size(); i += CHUNK) {
            List<Function> chunk = all.subList(i, Math.min(all.size(), i + CHUNK));
            DecompilerCallback<Result> cb = new DecompilerCallback<Result>(currentProgram, configurer) {
                @Override
                public Result process(DecompileResults res, TaskMonitor m) {
                    Function f = res.getFunction();
                    if (res.decompileCompleted() && res.getDecompiledFunction() != null) {
                        return new Result(f, res.getDecompiledFunction().getC(), true);
                    }
                    return new Result(f, "// DECOMPILE FAILED: " + res.getErrorMessage() + "\n", false);
                }
            };
            cb.setTimeout(120);
            List<Result> results = ParallelDecompiler.decompileFunctions(cb, chunk, monitor);
            cb.dispose();

            // group and append
            Map<String, StringBuilder> groups = new TreeMap<>();
            for (Result r : results) {
                if (r == null) continue;
                Function f = r.f;
                StringBuilder sb = groups.computeIfAbsent(groupFile(f), k -> new StringBuilder());
                String sig;
                try { sig = f.getSignature().getPrototypeString(); } catch (Exception e) { sig = f.getName(); }
                sb.append("// ============================================================\n");
                sb.append(String.format("// %s  size=0x%x  %s\n", f.getEntryPoint(), f.getBody().getNumAddresses(), f.getName(true)));
                sb.append("// ").append(sig).append("\n");
                sb.append(r.text);
                if (!r.text.endsWith("\n")) sb.append("\n");
                sb.append("\n");
                if (!r.ok) failed++;
            }
            for (Map.Entry<String, StringBuilder> e : groups.entrySet()) {
                File out = new File(outDir, e.getKey());
                out.getParentFile().mkdirs();
                try (Writer w = new OutputStreamWriter(new FileOutputStream(out, true), StandardCharsets.UTF_8)) {
                    w.write(e.getValue().toString());
                }
            }
            done += chunk.size();
            long el = (System.currentTimeMillis() - t0) / 1000;
            println(String.format("pseudo/: %d/%d (%d failed) %ds elapsed", done, all.size(), failed, el));
            if (monitor.isCancelled()) break;
        }
        println("pseudo/: done, " + failed + " failures");
    }

    // ---------------------------------------------------------------- .gzf
    private void exportGzf() throws Exception {
        File gzf = new File(outDir, currentProgram.getName() + ".gzf");
        GzfExporter ex = new GzfExporter();
        // DomainFile variant packs the saved database; run this script in a separate
        // "-process -noanalysis" pass after analysis has been saved.
        boolean ok = ex.export(gzf, currentProgram.getDomainFile(), monitor);
        println("gzf export " + (ok ? "ok" : "FAILED") + ": " + gzf + " (" + gzf.length() + " bytes)");
    }
}
