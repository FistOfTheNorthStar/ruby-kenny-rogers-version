require "bundler/setup"
require "i18n"

I18n.available_locales = [:en, :fr, :ru]
I18n.backend.store_translations(:en, hello: "Hello %{name}", apples: { zero: "no apples", one: "one apple", other: "%{count} apples" },
  nested: { deep: { key: "deep value" } }, date: { formats: { default: "%Y-%m-%d", short: "%b %d" }, abbr_month_names: [nil] + %w[Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec] })
I18n.backend.store_translations(:fr, hello: "Bonjour %{name}", apples: { one: "une pomme", other: "%{count} pommes" })
I18n.backend.store_translations(:ru, apples: { one: "%{count} яблоко", few: "%{count} яблока", many: "%{count} яблок", other: "%{count} яблока" })
I18n.default_locale = :en

p I18n.t(:hello, name: "Kenny"), I18n.t("nested.deep.key"), I18n.t(:missing), I18n.t(:missing, default: "fallback")
p [0, 1, 5].map { |n| I18n.t(:apples, count: n) }
I18n.with_locale(:fr) { p I18n.t(:hello, name: "Kenny"), I18n.t(:apples, count: 1), I18n.t(:apples, count: 3), I18n.locale }
require "i18n/backend/pluralization"
I18n::Backend::Simple.include(I18n::Backend::Pluralization)
I18n.backend.store_translations(:ru, i18n: { plural: { rule: ->(n) { n % 10 == 1 && n % 100 != 11 ? :one : [2, 3, 4].include?(n % 10) && ![12, 13, 14].include?(n % 100) ? :few : :many } } })
I18n.with_locale(:ru) { p [1, 2, 5, 21].map { |n| I18n.t(:apples, count: n) } }
p I18n.l(Date.new(2024, 2, 29)), I18n.l(Date.new(2024, 2, 29), format: :short)
p I18n.exists?(:hello), I18n.exists?(:nope), I18n.t(:nested), I18n.t([:hello, "nested.deep.key"], name: "x")
begin
  I18n.t!(:nope)
rescue I18n::MissingTranslationData => e
  p e.class, e.message
end
p I18n.transliterate("Ærøskøbing Café"), I18n.interpolate("%{a}-%{b}", a: 1, b: 2), I18n.normalize_keys(:en, "a.b", :c)
begin
  I18n.locale = :de
rescue I18n::InvalidLocale => e
  p e.message
end
require "i18n/backend/fallbacks"
I18n::Backend::Simple.include(I18n::Backend::Fallbacks)
I18n.fallbacks = I18n::Locale::Fallbacks.new(fr: :en)
I18n.with_locale(:fr) { p I18n.t("nested.deep.key") }
chain = I18n::Backend::Chain.new(I18n::Backend::Simple.new, I18n.backend)
chain.backends.first.store_translations(:en, hello: "Chained %{name}")
p chain.translate(:en, :hello, name: "c"), chain.translate(:en, "nested.deep.key")
