# A Rails (ActionPack/ActionView 5.2) HTML app: ERB templates, a partial, helpers.
# Templates are read from ./views at runtime and compiled by Kenny's embedded JIT.
require "action_controller"
require "action_view"

class PagesController < ActionController::Base
  prepend_view_path File.join(__dir__, "views")
  layout "application"

  def show
    @title = "Kenny"
    @name = params[:name]
    @items = %w[fast native]
  end
end

ROUTES = ActionDispatch::Routing::RouteSet.new
ROUTES.draw { get "/hi/:name", to: "pages#show" }
