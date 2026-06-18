# C-MECH transcriptional corroboration (ED Fig 3a): WRN-KO RNA-seq GSEA vs
# Hallmark (D56 wrapped-R, fgsea). Inputs (D53 multi-file): [1] GSE126464 STAR
# counts (genes x 12 samples, gz), [2] Hallmark .gmt (Collection profile).
cp <- .Call("r_eigon_materialized_path", eigenius_inputs[[1]])
gp <- .Call("r_eigon_materialized_path", eigenius_inputs[[2]])
suppressMessages({ library(limma); library(fgsea) })
df <- read.csv(gzfile(cp), check.names = FALSE, stringsAsFactors = FALSE)
g <- df[[1]]; ok <- !duplicated(g); M <- as.matrix(df[ok, -1]); rownames(M) <- g[ok]
samp <- colnames(M)
cond <- factor(ifelse(grepl("Ch2-2", samp), "control", "KO"), levels = c("control", "KO"))
cell <- factor(ifelse(grepl("^OVK18", samp), "OVK18", "SW48"))
keep <- rowSums(M >= 10) >= 6; M <- M[keep, ]
design <- model.matrix(~ cell + cond)            # last coef = WRN-KO vs control
fit <- eBayes(lmFit(voom(M, design), design))
tt  <- topTable(fit, coef = ncol(design), number = Inf, sort.by = "none")
ranks <- sort(setNames(tt$t, rownames(tt)))
pw <- gmtPathways(gp)
set.seed(1)
res <- fgsea(pw, ranks, minSize = 15, maxSize = 500)
gp_ <- function(p) res[pathway == p]
g2m <- gp_("HALLMARK_G2M_CHECKPOINT"); e2f <- gp_("HALLMARK_E2F_TARGETS")
p53 <- gp_("HALLMARK_P53_PATHWAY");    apo <- gp_("HALLMARK_APOPTOSIS")
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:gsea_mech:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:nes_g2m_checkpoint", g2m$NES)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:padj_g2m_checkpoint", g2m$padj)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:nes_e2f_targets", e2f$NES)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:nes_p53_pathway", p53$NES)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:padj_p53_pathway", p53$padj)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:nes_apoptosis", apo$NES)
# WRN-KO depletes proliferation signatures (G2M/E2F down) AND activates the p53
# response (up) -- the transcriptional signature of cell-cycle arrest.
if (g2m$NES < 0 && g2m$padj < 0.05 && p53$NES > 0 && p53$padj < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:CausesCellCycleArrest", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)
