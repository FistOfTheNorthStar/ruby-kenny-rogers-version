# A minimal Rails (ActionPack 5.2) JSON API, compiled by Kenny.
require "action_controller"

class HelloController < ActionController::API
  def show
    render json: { hello: params[:name], upcased: params[:name].upcase }
  end

  def create
    render json: { got: params[:item] }, status: :created
  end
end

ROUTES = ActionDispatch::Routing::RouteSet.new
ROUTES.draw do
  get "/hello/:name", to: "hello#show"
  post "/items", to: "hello#create"
end
