//! Play with Friends' text in every language the game has: `assets/lang/<code>.json`, which
//! horizon-official registers with the game (`horizon_platform::lang`).
use horizon_platform::lang::Files;

/// Every file in `assets/lang`, as `(code, JSON)`.
pub const LANG: &Files = &[
    ("af_za", include_str!("../../assets/lang/af_za.json")),
    ("ar_sa", include_str!("../../assets/lang/ar_sa.json")),
    ("az_az", include_str!("../../assets/lang/az_az.json")),
    ("ba_ru", include_str!("../../assets/lang/ba_ru.json")),
    ("be_by", include_str!("../../assets/lang/be_by.json")),
    ("bg_bg", include_str!("../../assets/lang/bg_bg.json")),
    ("br_fr", include_str!("../../assets/lang/br_fr.json")),
    ("bs_ba", include_str!("../../assets/lang/bs_ba.json")),
    ("ca_es", include_str!("../../assets/lang/ca_es.json")),
    ("cs_cz", include_str!("../../assets/lang/cs_cz.json")),
    ("cv_cu", include_str!("../../assets/lang/cv_cu.json")),
    ("cy_gb", include_str!("../../assets/lang/cy_gb.json")),
    ("da_dk", include_str!("../../assets/lang/da_dk.json")),
    ("de_de", include_str!("../../assets/lang/de_de.json")),
    ("el_gr", include_str!("../../assets/lang/el_gr.json")),
    ("en_us", include_str!("../../assets/lang/en_us.json")),
    ("eo_uy", include_str!("../../assets/lang/eo_uy.json")),
    ("es_es", include_str!("../../assets/lang/es_es.json")),
    ("et_ee", include_str!("../../assets/lang/et_ee.json")),
    ("eu_es", include_str!("../../assets/lang/eu_es.json")),
    ("fa_ir", include_str!("../../assets/lang/fa_ir.json")),
    ("fi_fi", include_str!("../../assets/lang/fi_fi.json")),
    ("fil_ph", include_str!("../../assets/lang/fil_ph.json")),
    ("fo_fo", include_str!("../../assets/lang/fo_fo.json")),
    ("fr_fr", include_str!("../../assets/lang/fr_fr.json")),
    ("fur_it", include_str!("../../assets/lang/fur_it.json")),
    ("fy_nl", include_str!("../../assets/lang/fy_nl.json")),
    ("ga_ie", include_str!("../../assets/lang/ga_ie.json")),
    ("gd_gb", include_str!("../../assets/lang/gd_gb.json")),
    ("gl_es", include_str!("../../assets/lang/gl_es.json")),
    ("haw_us", include_str!("../../assets/lang/haw_us.json")),
    ("he_il", include_str!("../../assets/lang/he_il.json")),
    ("hi_in", include_str!("../../assets/lang/hi_in.json")),
    ("hr_hr", include_str!("../../assets/lang/hr_hr.json")),
    ("hu_hu", include_str!("../../assets/lang/hu_hu.json")),
    ("hy_am", include_str!("../../assets/lang/hy_am.json")),
    ("id_id", include_str!("../../assets/lang/id_id.json")),
    ("ig_ng", include_str!("../../assets/lang/ig_ng.json")),
    ("is_is", include_str!("../../assets/lang/is_is.json")),
    ("it_it", include_str!("../../assets/lang/it_it.json")),
    ("ja_jp", include_str!("../../assets/lang/ja_jp.json")),
    ("ka_ge", include_str!("../../assets/lang/ka_ge.json")),
    ("kk_kz", include_str!("../../assets/lang/kk_kz.json")),
    ("kn_in", include_str!("../../assets/lang/kn_in.json")),
    ("ko_kr", include_str!("../../assets/lang/ko_kr.json")),
    ("ky_kg", include_str!("../../assets/lang/ky_kg.json")),
    ("la_la", include_str!("../../assets/lang/la_la.json")),
    ("lb_lu", include_str!("../../assets/lang/lb_lu.json")),
    ("li_li", include_str!("../../assets/lang/li_li.json")),
    ("lmo", include_str!("../../assets/lang/lmo.json")),
    ("lo_la", include_str!("../../assets/lang/lo_la.json")),
    ("lt_lt", include_str!("../../assets/lang/lt_lt.json")),
    ("lv_lv", include_str!("../../assets/lang/lv_lv.json")),
    ("mk_mk", include_str!("../../assets/lang/mk_mk.json")),
    ("mn_mn", include_str!("../../assets/lang/mn_mn.json")),
    ("ms_my", include_str!("../../assets/lang/ms_my.json")),
    ("mt_mt", include_str!("../../assets/lang/mt_mt.json")),
    ("nl_nl", include_str!("../../assets/lang/nl_nl.json")),
    ("no_no", include_str!("../../assets/lang/no_no.json")),
    ("oc_fr", include_str!("../../assets/lang/oc_fr.json")),
    ("pl_pl", include_str!("../../assets/lang/pl_pl.json")),
    ("pt_br", include_str!("../../assets/lang/pt_br.json")),
    ("pt_pt", include_str!("../../assets/lang/pt_pt.json")),
    ("ro_ro", include_str!("../../assets/lang/ro_ro.json")),
    ("ru_ru", include_str!("../../assets/lang/ru_ru.json")),
    ("sah_sah", include_str!("../../assets/lang/sah_sah.json")),
    ("se_no", include_str!("../../assets/lang/se_no.json")),
    ("sk_sk", include_str!("../../assets/lang/sk_sk.json")),
    ("sl_si", include_str!("../../assets/lang/sl_si.json")),
    ("so_so", include_str!("../../assets/lang/so_so.json")),
    ("sq_al", include_str!("../../assets/lang/sq_al.json")),
    ("sr_cs", include_str!("../../assets/lang/sr_cs.json")),
    ("sr_sp", include_str!("../../assets/lang/sr_sp.json")),
    ("sv_se", include_str!("../../assets/lang/sv_se.json")),
    ("szl", include_str!("../../assets/lang/szl.json")),
    ("ta_in", include_str!("../../assets/lang/ta_in.json")),
    ("th_th", include_str!("../../assets/lang/th_th.json")),
    ("tr_tr", include_str!("../../assets/lang/tr_tr.json")),
    ("tt_ru", include_str!("../../assets/lang/tt_ru.json")),
    ("uk_ua", include_str!("../../assets/lang/uk_ua.json")),
    ("ur_pk", include_str!("../../assets/lang/ur_pk.json")),
    ("uz_uz", include_str!("../../assets/lang/uz_uz.json")),
    ("vec_it", include_str!("../../assets/lang/vec_it.json")),
    ("vi_vn", include_str!("../../assets/lang/vi_vn.json")),
    ("yi_de", include_str!("../../assets/lang/yi_de.json")),
    ("yo_ng", include_str!("../../assets/lang/yo_ng.json")),
    ("zh_cn", include_str!("../../assets/lang/zh_cn.json")),
];

#[cfg(test)]
mod tests {
    use super::LANG;

    #[test]
    fn every_lang_file_is_registered_under_its_own_code() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lang");
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".json").map(str::to_owned))
            .collect();
        files.sort();
        let codes: Vec<&str> = LANG.iter().map(|(code, _)| *code).collect();
        assert_eq!(codes, files);
        for (code, json) in LANG {
            let body = std::fs::read_to_string(dir.join(format!("{code}.json"))).unwrap();
            assert_eq!(*json, body, "{code}");
        }
    }
}
