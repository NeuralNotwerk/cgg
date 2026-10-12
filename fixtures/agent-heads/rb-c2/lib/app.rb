class App
  def go
    Jobs::Runner.new.spawn_job(1)
    Jobs::Runner.kick(2)
  end
end
